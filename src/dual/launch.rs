//! Linux launcher preflight. Invoked only on an explicit `launch` command.
use super::session::LaunchSpec;
use std::{collections::BTreeMap, io::Read, path::PathBuf, sync::Arc};

const CHILD_ENV: &[&str] = &[
    "HOME",
    "PATH",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "COLORTERM",
    "LANG",
    "LC_ALL",
    "TZ",
    "TMPDIR",
];

/// A launch that passed every preflight check and has not touched the
/// process environment yet.
pub struct Prepared {
    spec: LaunchSpec,
    proxy_only: BTreeMap<String, String>,
}

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Prepared([environment redacted])")
    }
}

/// Production launch: preflight, scoped environment, fixed upstreams.
///
/// # Safety
/// Call only from the CLI main thread before starting any threads: this scopes
/// the process environment for the fixed provider runtime.
pub unsafe fn run(client: Option<PathBuf>, resume: Option<String>) -> anyhow::Result<i32> {
    let prepared = unsafe { preflight(client, resume)? };
    unsafe { scope_environment(&prepared) };
    let router = router(
        crate::providers::anthropic::AnthropicProvider::new(),
        crate::providers::codex::CodexProvider::new(),
    );
    run_prepared(prepared, router)
}

/// Validates configuration, the client executable and its exact version.
/// Starts no listener and leaves the environment unchanged.
///
/// # Safety
/// Call only from the main thread before starting any threads: the version
/// check spawns the client with the current environment.
pub unsafe fn preflight(
    client: Option<PathBuf>,
    resume: Option<String>,
) -> anyhow::Result<Prepared> {
    anyhow::ensure!(cfg!(target_os = "linux"), "launch supports Linux/WSL2 only");
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| {
            anyhow::anyhow!("absolute HOME required; no listener or client was started")
        })?;
    let report = super::doctor::current()?;
    let conflicts: Vec<_> = report
        .findings
        .iter()
        .filter(|f| {
            (matches!(f.status, "blocked" | "conflict")
                || (f.status == "review"
                    && [
                        "apiKeyHelper ",
                        "model ",
                        "modelSettings ",
                        "availableModels ",
                        "deniedModels ",
                        "agents ",
                        "autoCompactWindow ",
                    ]
                    .iter()
                    .any(|key| f.detail.starts_with(key)))
                || (f.status == "unknown" && (f.scope == "user" || f.scope.starts_with("project"))))
                && !f.detail.starts_with("CCP_CONFIG_DIR ")
        })
        .collect();
    anyhow::ensure!(
        conflicts.is_empty(),
        "configuration conflicts detected; run doctor and resolve reported keys (values redacted); no listener or client was started"
    );
    for path in [
        PathBuf::from("/etc/claude-code/managed-settings.json"),
        PathBuf::from("/etc/claude-code/managed-settings.d"),
        home.join(".claude/managed-settings.json"),
        home.join(".claude/managed-settings.d"),
    ] {
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            _ => anyhow::bail!(
                "managed configuration present or inaccessible; effective policy requires review; no listener or client was started"
            ),
        }
    }
    let client = client.or_else(|| {
        let conventional = home.join(".local/bin/claude");
        if conventional.is_file() { return Some(conventional); }
        std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p)
            .filter(|p| p.is_absolute()).map(|p| p.join("claude")).find(|p| p.is_file()))
    }).ok_or_else(|| anyhow::anyhow!("Linux Claude executable not found; use --claude /absolute/path; no listener or client was started"))?;
    anyhow::ensure!(
        client.is_absolute() && client.is_file(),
        "Claude executable must be an existing absolute path; no listener or client was started"
    );
    let mut inherited = BTreeMap::new();
    let mut proxy_only = BTreeMap::new();
    for (key, value) in std::env::vars_os() {
        let key = key
            .into_string()
            .map_err(|_| anyhow::anyhow!("invalid environment encoding; key and value redacted"))?;
        let value = value
            .into_string()
            .map_err(|_| anyhow::anyhow!("invalid environment encoding; value redacted"))?;
        inherited.insert(key.clone(), value.clone());
        if CHILD_ENV.contains(&key.as_str())
            || ["XDG_CONFIG_HOME", "XDG_STATE_HOME", "CCP_CONFIG_DIR"].contains(&key.as_str())
        {
            proxy_only.insert(key, value);
        }
    }
    let spec = LaunchSpec::new(
        &client,
        &std::env::current_dir()?,
        resume.as_deref(),
        inherited.clone(),
    )?;
    let mut version_command = std::process::Command::new(&client);
    version_command
        .env_clear()
        .envs(&inherited)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        version_command.process_group(0);
    }
    let mut child = version_command.spawn()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        if let Some(status) = super::session::peek_exit_code(&mut child)? {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    anyhow::ensure!(
        status.is_some(),
        "Claude version check timed out; no listener was started"
    );
    let mut version = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .take(512)
        .read_to_end(&mut version)?;
    anyhow::ensure!(
        status.unwrap() == 0
            && version == format!("{} (Claude Code)\n", super::CLIENT_VERSION).as_bytes(),
        "Claude version is not validated {}; output redacted; no listener was started",
        super::CLIENT_VERSION
    );
    Ok(Prepared { spec, proxy_only })
}

/// Removes inherited provider endpoints, proxies, logging, auth and model
/// overrides and selects the fixed dual-session defaults. Nothing is written
/// back to the user's shell.
///
/// # Safety
/// No other thread may exist: this mutates the process environment.
pub unsafe fn scope_environment(prepared: &Prepared) {
    for (key, _) in std::env::vars_os() {
        unsafe {
            std::env::remove_var(key);
        }
    }
    for (key, value) in &prepared.proxy_only {
        unsafe {
            std::env::set_var(key, value);
        }
    }
    crate::config::use_dual_session_defaults();
}

/// The only router a dual session serves: native Claude and Codex, nothing
/// else. Production passes fixed-upstream providers.
pub fn router(
    anthropic: crate::providers::anthropic::AnthropicProvider,
    codex: crate::providers::codex::CodexProvider,
) -> axum::Router {
    let registry = crate::registry::Registry::from_providers(
        crate::config::AliasProvider::Anthropic,
        vec![
            Arc::new(anthropic) as Arc<dyn crate::provider::Provider>,
            Arc::new(codex),
        ],
    );
    crate::server::app_with_features(
        Arc::new(registry),
        None,
        crate::server::AppFeatures::default(),
    )
}

/// Supervises the session on the calling (main) thread until the child exits
/// or the launcher receives a termination signal.
pub fn run_prepared(prepared: Prepared, router: axum::Router) -> anyhow::Result<i32> {
    let spec = prepared.spec;
    let hangup_ignored = sighup_ignored();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    eprintln!(
        "claude-code-dual: parent Opus 5.5, default worker Astra [1m]; normal project hooks, permissions and MCP retained. Remote policy and upstream capacity remain unverified."
    );
    runtime.block_on(async move {
        // Handlers are registered before the child starts, so an early
        // termination signal still runs the session cleanup.
        let stop = StopSignals::install(hangup_ignored)?;
        super::session::supervise(&spec, router, stop.wait()).await
    })
}

/// Whether SIGHUP was ignored when the launcher started, as under `nohup`;
/// then no hangup handler is installed.
fn sighup_ignored() -> bool {
    #[cfg(unix)]
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        libc::sigaction(libc::SIGHUP, std::ptr::null(), &mut action) == 0
            && action.sa_sigaction == libc::SIG_IGN
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Termination signal handlers, registered when created.
struct StopSignals {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(unix)]
    hangup: Option<tokio::signal::unix::Signal>,
}

impl StopSignals {
    fn install(hangup_ignored: bool) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                interrupt: signal(SignalKind::interrupt())?,
                terminate: signal(SignalKind::terminate())?,
                hangup: if hangup_ignored {
                    None
                } else {
                    Some(signal(SignalKind::hangup())?)
                },
            })
        }
        #[cfg(not(unix))]
        {
            let _ = hangup_ignored;
            Ok(Self {})
        }
    }

    /// Resolves with the conventional exit code of the first signal.
    async fn wait(self) -> i32 {
        #[cfg(unix)]
        {
            let Self {
                mut interrupt,
                mut terminate,
                mut hangup,
            } = self;
            let hangup = async {
                match hangup.as_mut() {
                    Some(hangup) => {
                        hangup.recv().await;
                    }
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                _ = interrupt.recv() => 130,
                _ = terminate.recv() => 143,
                _ = hangup => 129,
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
            130
        }
    }
}
