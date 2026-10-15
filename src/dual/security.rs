use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rand::RngCore;
use std::{fmt, sync::Arc};

pub const CAPABILITY_HEADER: &str = "x-claude-dual-capability";

/// In-memory local secret. Never serialize or place in URLs/arguments.
#[derive(Clone)]
pub struct Capability(Arc<String>);

impl Capability {
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(Arc::new(bytes.iter().map(|b| format!("{b:02x}")).collect()))
    }

    /// Intended only for the child's scoped ANTHROPIC_CUSTOM_HEADERS environment.
    pub fn child_header(&self) -> String {
        format!("{CAPABILITY_HEADER}: {}", self.0)
    }

    fn matches(&self, value: &[u8]) -> bool {
        let expected = self.0.as_bytes();
        if value.len() != expected.len() {
            return false;
        }
        value
            .iter()
            .zip(expected)
            .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

impl fmt::Debug for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Capability([REDACTED])")
    }
}

/// Must be the outermost layer so rejected requests never reach logging/routing.
/// The supplied inner router must use fixed upstreams; this layer is not an SSRF proxy.
pub fn protect(router: Router, capability: Capability) -> Router {
    router.layer(middleware::from_fn_with_state(capability, authorize))
}

async fn authorize(
    State(capability): State<Capability>,
    mut request: Request,
    next: Next,
) -> Response {
    let mut values = request.headers().get_all(CAPABILITY_HEADER).iter();
    let authorized = values
        .next()
        .is_some_and(|v| capability.matches(v.as_bytes()))
        && values.next().is_none();
    if !authorized || request.headers().contains_key("origin") {
        return (StatusCode::UNAUTHORIZED, "local capability required").into_response();
    }
    request.headers_mut().remove(CAPABILITY_HEADER);
    let allowed = matches!(
        (request.method(), request.uri().path()),
        (&Method::GET, "/healthz" | "/v1/models")
            | (&Method::POST, "/v1/messages" | "/v1/messages/count_tokens")
    );
    if !allowed || request.uri().query().is_some_and(|q| q != "beta=true") {
        return (StatusCode::NOT_FOUND, "route unavailable").into_response();
    }
    if request.method() == Method::POST {
        let (parts, body) = request.into_parts();
        let Ok(bytes) = axum::body::to_bytes(body, 32 * 1024 * 1024).await else {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        };
        let Ok(document) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return (StatusCode::BAD_REQUEST, "invalid JSON").into_response();
        };
        let permitted = document
            .get("model")
            .and_then(|v| v.as_str())
            .is_some_and(|m| {
                let m = crate::registry::normalize_incoming_model(m);
                m.starts_with("claude-")
                    || crate::registry::is_anthropic_alias(&m)
                    || m == "gpt-6-astra"
            });
        if !permitted {
            return (
                StatusCode::BAD_REQUEST,
                "model outside dual-session contract",
            )
                .into_response();
        }
        request = Request::from_parts(parts, Body::from(bytes));
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::Request, routing::any};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    #[tokio::test]
    async fn rejects_before_dispatch_and_strips_local_secret() {
        let hits = Arc::new(AtomicUsize::new(0));
        let observed = hits.clone();
        let app = Router::new().fallback(any(move |request: Request<Body>| {
            let observed = observed.clone();
            async move {
                assert!(!request.headers().contains_key(CAPABILITY_HEADER));
                assert_eq!(request.headers()["authorization"], "Bearer synthetic-oauth");
                assert_eq!(request.headers()["anthropic-beta"], "synthetic-beta");
                observed.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }
        }));
        let cap = Capability::generate();
        let router = protect(app, cap.clone());
        for token in [None, Some("wrong"), Some("")] {
            let mut request = Request::post("/v1/messages");
            if let Some(token) = token {
                request = request.header(CAPABILITY_HEADER, token);
            }
            assert_eq!(
                router
                    .clone()
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        for (path, model, expected) in [
            ("/v1/messages", "gpt-6-astra[1m]", StatusCode::OK),
            (
                "/v1/messages/count_tokens?beta=true",
                "opus",
                StatusCode::OK,
            ),
            ("/v1/messages", "cursor", StatusCode::BAD_REQUEST),
            ("/v1/responses", "gpt-6-astra", StatusCode::NOT_FOUND),
            ("/v1/messages?secret=canary", "opus", StatusCode::NOT_FOUND),
        ] {
            let request = Request::post(path)
                .header(CAPABILITY_HEADER, cap.0.as_str())
                .header("authorization", "Bearer synthetic-oauth")
                .header("anthropic-beta", "synthetic-beta")
                .body(Body::from(serde_json::json!({"model":model}).to_string()))
                .unwrap();
            assert_eq!(
                router.clone().oneshot(request).await.unwrap().status(),
                expected
            );
        }
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        for origin in [false, true] {
            let mut req = Request::post("/v1/messages").header(CAPABILITY_HEADER, cap.0.as_str());
            req = if origin {
                req.header("origin", "https://example.invalid")
            } else {
                req.header(CAPABILITY_HEADER, cap.0.as_str())
            };
            assert_eq!(
                router
                    .clone()
                    .oneshot(req.body(Body::empty()).unwrap())
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert!(!format!("{cap:?}").contains(cap.0.as_str()));
        assert_ne!(cap.0, Capability::generate().0);
    }
}
