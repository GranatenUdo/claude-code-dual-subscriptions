use assert_cmd::Command;
use predicates::str::contains;

fn command(config: &str) -> (tempfile::TempDir, Command) {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("config.json"), config).unwrap();
    let mut cmd = Command::cargo_bin("claude-code-proxy").unwrap();
    cmd.env_clear()
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .env("CCP_CONFIG_DIR", home.path());
    (home, cmd)
}

#[test]
fn favorites_are_advertised_without_credentials() {
    let (_home, mut cmd) = command(
        r#"{"openrouter":{"models":["vendor/model:free","openrouter/auto","vendor/model:free"]}}"#,
    );
    let output = cmd.arg("models").output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("openrouter: openrouter/openrouter/auto, openrouter/vendor/model:free"),
        "{text}"
    );
    assert_eq!(text.matches("openrouter/vendor/model:free").count(), 1);
}

#[test]
fn invalid_openrouter_block_does_not_reset_native_alias_provider() {
    for block in ["42", r#"{"models":42}"#, r#"{"models":["bad"]}"#] {
        let (_home, mut cmd) = command(&format!(
            r#"{{"aliasProvider":"anthropic","openrouter":{block}}}"#
        ));
        cmd.arg("models")
            .assert()
            .success()
            .stdout(contains("anthropic:"));
    }
}

#[test]
fn status_reports_key_source_without_exposing_key_or_url() {
    let (_home, mut cmd) = command(r#"{"openrouter":{"apiKey":"file-secret"}}"#);
    cmd.env("OPENROUTER_API_KEY", "fallback-secret")
        .env("CCP_OPENROUTER_API_KEY", "preferred-secret")
        .args(["openrouter", "auth", "status"])
        .assert()
        .success()
        .stdout(contains("CCP_OPENROUTER_API_KEY"));
    let (_home, mut cmd) =
        command(r#"{"openrouter":{"baseUrl":"https://user:url-secret@example.test"}}"#);
    let out = cmd.args(["openrouter", "auth", "status"]).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success());
    assert!(!text.contains("url-secret"));
    assert!(text.contains("OpenRouter"));
}
