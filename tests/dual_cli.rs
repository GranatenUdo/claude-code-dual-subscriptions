use assert_cmd::Command;

#[cfg(target_os = "linux")]
#[test]
fn missing_client_fails_without_creating_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = Command::cargo_bin("claude-code-dual").unwrap();
    command
        .env_clear()
        .env("HOME", dir.path())
        .current_dir(dir.path())
        .arg("launch")
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "no listener or client was started",
        ));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn launch_runs_scoped_fake_client_and_propagates_exit() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let client = dir.path().join("claude");
    std::fs::write(&client, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo '2.1.300 (Claude Code)'; exit 0; fi\n[ \"$CLAUDE_CODE_SUBAGENT_MODEL\" = 'gpt-6-astra[1m]' ] || exit 80\n[ -z \"${ANTHROPIC_AUTH_TOKEN+x}\" ] || exit 81\n[ -z \"${CLAUDE_CODE_SUBAGENT_MODEL_FORCE+x}\" ] || exit 82\n[ -n \"$ANTHROPIC_CUSTOM_HEADERS\" ] || exit 83\n[ \"$1\" = --model ] || exit 84\nprintf '%s' \"$ANTHROPIC_BASE_URL\" > endpoint\nexit 23\n").unwrap();
    std::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = Command::cargo_bin("claude-code-dual")
        .unwrap()
        .env_clear()
        .env("HOME", dir.path())
        .env("PATH", "/usr/bin:/bin")
        .current_dir(dir.path())
        .args(["launch", "--claude"])
        .arg(&client)
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(23),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let endpoint = std::fs::read_to_string(dir.path().join("endpoint")).unwrap();
    assert!(endpoint.starts_with("http://127.0.0.1:"));
    assert!(std::net::TcpStream::connect(endpoint.trim_start_matches("http://")).is_err());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("x-claude-dual-capability:"));
}

#[cfg(target_os = "linux")]
#[test]
fn previous_client_version_is_refused_before_listener_start() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let client = dir.path().join("claude");
    std::fs::write(
        &client,
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo '2.1.299 (Claude Code)'; exit 0; fi\ntouch executed\n",
    )
    .unwrap();
    std::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = Command::cargo_bin("claude-code-dual")
        .unwrap()
        .env_clear()
        .env("HOME", dir.path())
        .env("PATH", "/usr/bin:/bin")
        .current_dir(dir.path())
        .args(["launch", "--claude"])
        .arg(&client)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!dir.path().join("executed").exists());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("not validated 2.1.300"), "{stderr}");
    assert!(stderr.contains("no listener was started"), "{stderr}");
}

#[cfg(target_os = "linux")]
#[test]
fn auth_conflict_fails_before_client_execution() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let client = dir.path().join("claude");
    std::fs::write(&client, "#!/bin/sh\ntouch executed\n").unwrap();
    std::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = Command::cargo_bin("claude-code-dual")
        .unwrap()
        .env_clear()
        .env("HOME", dir.path())
        .env("ANTHROPIC_AUTH_TOKEN", "SECRET-CANARY")
        .current_dir(dir.path())
        .args(["launch", "--claude"])
        .arg(client)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!dir.path().join("executed").exists());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("CANARY"));
}

#[cfg(target_os = "linux")]
#[test]
fn configured_auth_and_model_policy_fail_before_execution() {
    use std::os::unix::fs::PermissionsExt;
    for key in ["apiKeyHelper", "model", "availableModels", "deniedModels"] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".claude")).unwrap();
        std::fs::write(
            dir.path().join(".claude/settings.json"),
            serde_json::json!({key:"SECRET-CANARY"}).to_string(),
        )
        .unwrap();
        let client = dir.path().join("claude");
        std::fs::write(&client, "#!/bin/sh\ntouch executed\n").unwrap();
        std::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = Command::cargo_bin("claude-code-dual")
            .unwrap()
            .env_clear()
            .env("HOME", dir.path())
            .current_dir(dir.path())
            .args(["launch", "--claude"])
            .arg(client)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!dir.path().join("executed").exists());
        assert!(!String::from_utf8_lossy(&result.stderr).contains("CANARY"));
    }
}

#[cfg(unix)]
#[test]
fn doctor_non_utf8_environment_is_redacted_without_panicking() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let dir = tempfile::tempdir().unwrap();
    let mut command = Command::cargo_bin("claude-code-dual").unwrap();
    let result = command
        .env_clear()
        .env("HOME", dir.path())
        .current_dir(dir.path())
        .env(
            "UNRELATED_SECRET",
            OsString::from_vec(b"SECRET-CANARY-\xff".to_vec()),
        )
        .env(
            "ANTHROPIC_AUTH_TOKEN",
            OsString::from_vec(b"TOKEN-CANARY-\xff".to_vec()),
        )
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!combined.contains("CANARY"));
    assert!(!combined.contains("panicked"));
    assert!(combined.contains("ANTHROPIC_AUTH_TOKEN"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
