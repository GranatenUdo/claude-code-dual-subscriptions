//! Bounded, offline inspection. Never execute the client or open credential stores.
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_SETTINGS: u64 = 64 * 1024;
const KEYS: &[&str] = &[
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "ANTHROPIC_CUSTOM_HEADERS",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL_FORCE",
    "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
    "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
    "CLAUDE_CODE_DISABLE_1M_CONTEXT",
    "DISABLE_COMPACT",
    "CLAUDE_CONFIG_DIR",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "CCP_CONFIG_DIR",
];

pub fn conflicting_env_key(key: &str) -> bool {
    KEYS.contains(&key)
        || key.starts_with("ANTHROPIC_")
        || key.starts_with("CLAUDE_CODE_SUBAGENT_MODEL")
        || key.starts_with("CLAUDE_CODE_USE_")
        || matches!(key, "CLAUDE_CODE_SAFE_MODE" | "CLAUDE_CODE_SIMPLE")
}

#[derive(Debug, Serialize)]
pub struct Finding {
    pub scope: String,
    pub status: &'static str,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub product: &'static str,
    pub launch_supported: bool,
    pub client_version: &'static str,
    pub auth_status: &'static str,
    pub findings: Vec<Finding>,
}

fn finding(scope: &str, status: &'static str, detail: impl Into<String>) -> Finding {
    Finding {
        scope: scope.into(),
        status,
        detail: detail.into(),
    }
}

/// Only key names from a fixed allowlist are reported, never values or parse errors.
pub fn inspect_settings(path: &Path, scope: &str) -> Vec<Finding> {
    // Reject links anywhere in the path, including the settings directory.
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return vec![finding(
                    scope,
                    "blocked",
                    "symlinked settings path; not read",
                )];
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return vec![finding(scope, "unknown", "settings path inaccessible")],
            _ => {}
        }
    }
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return vec![finding(scope, "absent", "settings file absent")];
        }
        Err(_) => return vec![finding(scope, "unknown", "settings metadata inaccessible")],
    };
    if !meta.is_file() || meta.len() > MAX_SETTINGS {
        return vec![finding(
            scope,
            "blocked",
            "settings must be a regular file at most 64 KiB",
        )];
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let Ok(file) = options.open(path) else {
        return vec![finding(scope, "unknown", "settings unreadable")];
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return vec![finding(
            scope,
            "blocked",
            "settings changed during inspection",
        )];
    }
    let mut bytes = Vec::new();
    if file.take(MAX_SETTINGS + 1).read_to_end(&mut bytes).is_err()
        || bytes.len() as u64 > MAX_SETTINGS
    {
        return vec![finding(
            scope,
            "blocked",
            "settings read failed or size exceeded",
        )];
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return vec![finding(
            scope,
            "blocked",
            "invalid settings JSON; contents redacted",
        )];
    };
    if !value.is_object() {
        return vec![finding(scope, "blocked", "settings root must be an object")];
    }
    let mut out = Vec::new();
    if let Some(env) = value.get("env") {
        if let Some(env) = env.as_object() {
            for key in KEYS {
                if env.contains_key(*key) {
                    out.push(finding(
                        scope,
                        "conflict",
                        format!("{key} is configured; value redacted"),
                    ));
                }
            }
            if env
                .keys()
                .any(|key| conflicting_env_key(key) && !KEYS.contains(&key.as_str()))
            {
                out.push(finding(
                    scope,
                    "conflict",
                    "additional routing/auth environment override; key and value redacted",
                ));
            }
        } else {
            out.push(finding(scope, "blocked", "env must be an object"));
        }
    }
    for key in [
        "apiKeyHelper",
        "model",
        "modelSettings",
        "autoCompactWindow",
        "availableModels",
        "deniedModels",
        "agents",
        "hooks",
        "mcpServers",
        "enabledPlugins",
    ] {
        if value.get(key).is_some() {
            out.push(finding(
                scope,
                "review",
                format!("{key} is configured; contents not evaluated"),
            ));
        }
    }
    if out.is_empty() {
        out.push(finding(
            scope,
            "inspected",
            "no known conflict keys; this is not effective-policy verification",
        ));
    }
    out
}

pub fn inspect(cwd: &Path, home: Option<&Path>, env: &BTreeMap<String, String>) -> Report {
    let mut findings = Vec::new();
    for key in KEYS {
        if env.contains_key(*key) {
            findings.push(finding(
                "environment",
                "conflict",
                format!("{key} is set; value redacted"),
            ));
        }
    }
    if env
        .keys()
        .any(|key| conflicting_env_key(key) && !KEYS.contains(&key.as_str()))
    {
        findings.push(finding(
            "environment",
            "conflict",
            "additional routing/auth environment override; key and value redacted",
        ));
    }
    if let Some(home) = home {
        findings.extend(inspect_settings(
            &home.join(".claude/settings.json"),
            "user",
        ));
    } else {
        findings.push(finding("user", "unknown", "home unavailable"));
    }
    for (depth, directory) in cwd.ancestors().take(33).enumerate() {
        if depth == 32 {
            findings.push(finding(
                "project",
                "unknown",
                "ancestor inspection limit reached",
            ));
            break;
        }
        for name in ["settings.json", "settings.local.json"] {
            findings.extend(inspect_settings(
                &directory.join(".claude").join(name),
                &format!("project ancestor {depth} {name}"),
            ));
        }
    }
    findings.extend(inspect_settings(
        Path::new("/etc/claude-code/managed-settings.json"),
        "managed file",
    ));
    findings.push(finding("managed policy", "unknown", "remote policy, managed fragments and agent definitions not fully evaluated; do not infer absence of policy"));
    let client = env
        .get("PATH")
        .map(|p| {
            std::env::split_paths(p).any(|dir| {
                !dir.as_os_str().is_empty() && dir.is_absolute() && dir.join("claude").is_file()
            })
        })
        .unwrap_or(false);
    findings.push(finding(
        "client",
        if client { "found" } else { "unknown" },
        "PATH metadata only; client was not executed or authenticated",
    ));
    Report {
        product: "claude-code-dual",
        launch_supported: false,
        client_version: "unknown (not executed)",
        auth_status: "unknown (credentials not read)",
        findings,
    }
}

pub fn current() -> anyhow::Result<Report> {
    let mut env = BTreeMap::new();
    let mut invalid_path = false;
    for (key, value) in std::env::vars_os() {
        let Some(key) = key.to_str() else {
            continue;
        };
        if conflicting_env_key(key) {
            // Only presence is relevant: never decode or retain secret values.
            env.insert(key.to_owned(), String::new());
        } else if key == "PATH" {
            if let Some(value) = value.to_str() {
                env.insert(key.to_owned(), value.to_owned());
            } else {
                invalid_path = true;
            }
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut report = inspect(&std::env::current_dir()?, home.as_deref(), &env);
    if invalid_path {
        report.findings.push(finding(
            "client",
            "unknown",
            "PATH is not UTF-8; contents redacted",
        ));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_settings_and_environment_never_leak_values() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        fs::write(&file, r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"SECRET-CANARY","CLAUDE_CODE_SUBAGENT_MODEL_FORCE":"SECRET-CANARY"},"apiKeyHelper":"touch SECRET-CANARY"}"#).unwrap();
        let out = serde_json::to_string(&inspect_settings(&file, "test")).unwrap();
        assert!(!out.contains("SECRET-CANARY"));
        assert!(out.contains("ANTHROPIC_AUTH_TOKEN"));
        fs::write(&file, "SECRET-CANARY{invalid").unwrap();
        let out = serde_json::to_string(&inspect_settings(&file, "test")).unwrap();
        assert!(!out.contains("SECRET-CANARY"));
        assert!(out.contains("invalid settings"));
        fs::write(&file, vec![b'x'; MAX_SETTINGS as usize + 1]).unwrap();
        assert_eq!(inspect_settings(&file, "test")[0].status, "blocked");
    }
    #[cfg(unix)]
    #[test]
    fn settings_symlinks_are_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink("/dev/zero", &link).unwrap();
        assert_eq!(inspect_settings(&link, "test")[0].status, "blocked");
        assert_eq!(inspect_settings(dir.path(), "test")[0].status, "blocked");
    }
}
