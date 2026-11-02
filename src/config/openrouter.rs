use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Deliberately not Debug: this value owns a credential.
pub struct OpenRouterConfig {
    pub api_key: Option<String>,
    pub api_key_source: Option<&'static str>,
    pub base_url: reqwest::Url,
    pub models: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileOpenRouter {
    api_key: Option<String>,
    base_url: Option<String>,
    #[serde(default)]
    models: Vec<String>,
}

pub fn valid_openrouter_model(id: &str) -> bool {
    id.contains('/')
        && !id.chars().any(|c| c.is_whitespace() || c.is_control())
        && id.split('/').all(|part| !part.is_empty())
}

pub fn resolve_openrouter_config(
    env: &HashMap<String, String>,
    config_dir: &Path,
) -> Result<OpenRouterConfig, String> {
    let raw = super::read_file_config(config_dir).and_then(|file| file.openrouter);
    let file: FileOpenRouter = match raw {
        Some(raw) => serde_json::from_value(raw).map_err(|_| "Invalid OpenRouter configuration: expected apiKey/baseUrl strings and a models array of namespaced IDs".to_string())?,
        None => FileOpenRouter::default(),
    };
    let (api_key, api_key_source) = ["CCP_OPENROUTER_API_KEY", "OPENROUTER_API_KEY"]
        .into_iter()
        .find_map(|name| {
            env.get(name)
                .filter(|value| !value.is_empty())
                .map(|value| (Some(value.clone()), Some(name)))
        })
        .unwrap_or_else(|| {
            (
                file.api_key.filter(|value| !value.is_empty()),
                Some("config.json"),
            )
        });
    let api_key_source = api_key.as_ref().and(api_key_source);
    if api_key
        .as_ref()
        .is_some_and(|key| http::HeaderValue::from_str(&format!("Bearer {key}")).is_err())
    {
        return Err("Invalid OpenRouter API key header".into());
    }
    let base = env
        .get("CCP_OPENROUTER_BASE_URL")
        .filter(|v| !v.is_empty())
        .cloned()
        .or(file.base_url.filter(|v| !v.is_empty()))
        .unwrap_or_else(|| DEFAULT_BASE_URL.into());
    let base_url =
        reqwest::Url::parse(&base).map_err(|_| "Invalid OpenRouter base URL".to_string())?;
    let loopback = base_url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !(base_url.scheme() == "https" || (base_url.scheme() == "http" && loopback))
        || base_url.host_str().is_none()
        || !base_url.username().is_empty()
        || base_url.password().is_some()
        || base_url.query().is_some()
        || base_url.fragment().is_some()
    {
        return Err("Invalid OpenRouter base URL: use HTTPS (or loopback HTTP), without credentials, query, or fragment".into());
    }
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for id in file.models {
        if !valid_openrouter_model(&id) {
            return Err("Invalid OpenRouter models: use namespace/name without whitespace or empty segments".into());
        }
        if seen.insert(id.clone()) {
            models.push(id);
        }
    }
    Ok(OpenRouterConfig {
        api_key,
        api_key_source,
        base_url,
        models,
    })
}

pub fn openrouter_config() -> Result<OpenRouterConfig, String> {
    // Read only the provider's environment inputs, never enumerate credentials.
    let env = [
        "CCP_OPENROUTER_API_KEY",
        "OPENROUTER_API_KEY",
        "CCP_OPENROUTER_BASE_URL",
    ]
    .into_iter()
    .filter_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| (key.to_string(), value))
    })
    .collect();
    resolve_openrouter_config(&env, &crate::paths::config_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_empty_values_and_ordered_deduplication() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), r#"{"openrouter":{"apiKey":"file","baseUrl":"https://file.test/v1","models":["z/model","a/model:free","z/model"]}}"#).unwrap();
        let mut env = HashMap::new();
        let cfg = resolve_openrouter_config(&env, dir.path()).unwrap();
        assert_eq!(cfg.api_key.as_deref(), Some("file"));
        assert_eq!(cfg.models, ["z/model", "a/model:free"]);
        env.insert("OPENROUTER_API_KEY".into(), "fallback".into());
        env.insert("CCP_OPENROUTER_API_KEY".into(), "preferred".into());
        env.insert(
            "CCP_OPENROUTER_BASE_URL".into(),
            "https://env.test/api/v1".into(),
        );
        let cfg = resolve_openrouter_config(&env, dir.path()).unwrap();
        assert_eq!(cfg.api_key.as_deref(), Some("preferred"));
        assert_eq!(cfg.base_url.as_str(), "https://env.test/api/v1");
        env.insert("CCP_OPENROUTER_API_KEY".into(), "".into());
        assert_eq!(
            resolve_openrouter_config(&env, dir.path())
                .unwrap()
                .api_key
                .as_deref(),
            Some("fallback")
        );
        env.insert("OPENROUTER_API_KEY".into(), "".into());
        assert_eq!(
            resolve_openrouter_config(&env, dir.path())
                .unwrap()
                .api_key
                .as_deref(),
            Some("file")
        );
    }
    #[test]
    fn invalid_urls_and_model_lists_are_rejected_without_echoing_values() {
        let dir = tempfile::tempdir().unwrap();
        for url in [
            "http://remote.test",
            "https://secret@host.test",
            "https://host.test?secret",
            "https://host.test/#secret",
            "invalid-secret",
        ] {
            let env = HashMap::from([("CCP_OPENROUTER_BASE_URL".into(), url.into())]);
            let error = resolve_openrouter_config(&env, dir.path()).err().unwrap();
            assert!(!error.contains("secret"));
        }
        for url in [
            "http://127.0.0.1:1234/api/v1",
            "http://[::1]:1234/api/v1",
            "https://openrouter.ai/api/v1",
        ] {
            assert!(
                resolve_openrouter_config(
                    &HashMap::from([("CCP_OPENROUTER_BASE_URL".into(), url.into())]),
                    dir.path()
                )
                .is_ok()
            );
        }
        for id in [
            "",
            "bare",
            "vendor/",
            "/model",
            "vendor//model",
            "vendor/has space",
            "vendor/model\n",
        ] {
            assert!(!valid_openrouter_model(id));
        }
        for id in ["vendor/model:free", "openrouter/auto", "vendor/group/model"] {
            assert!(valid_openrouter_model(id));
        }
    }
}
