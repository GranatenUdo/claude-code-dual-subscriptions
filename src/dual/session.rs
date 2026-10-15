//! Scoped session contract and owned Linux child/server lifecycle.
use super::{PARENT_MODEL, WORKER_MODEL, security::Capability};
use std::{
    collections::BTreeMap,
    future::IntoFuture,
    path::{Path, PathBuf},
    process::Command,
};

/// Does not carry a capability; safe to inspect before creating the local secret.
pub struct LaunchSpec {
    client: PathBuf,
    cwd: PathBuf,
    resume: Option<String>,
    inherited: BTreeMap<String, String>,
}

impl std::fmt::Debug for LaunchSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LaunchSpec([environment redacted])")
    }
}

impl LaunchSpec {
    /// Pure validation, no execution or settings reads. Callers must separately
    /// validate effective settings and the real-client gate before execution.
    pub fn new(
        client: &Path,
        cwd: &Path,
        resume: Option<&str>,
        inherited: BTreeMap<String, String>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            client.is_absolute() && cwd.is_absolute(),
            "client and cwd must be absolute"
        );
        for key in inherited.keys() {
            anyhow::ensure!(
                !super::doctor::conflicting_env_key(key) || key == "CCP_CONFIG_DIR",
                "conflicting environment key present (values redacted)"
            );
        }
        if let Some(id) = resume {
            anyhow::ensure!(
                !id.is_empty()
                    && id.len() <= 128
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                "resume must be a session identifier, not a path or option"
            );
        }
        Ok(Self {
            client: client.into(),
            cwd: cwd.into(),
            resume: resume.map(str::to_owned),
            inherited,
        })
    }

    /// Never log the resulting Command: its environment includes a local secret.
    pub fn command(&self, port: u16, capability: &Capability) -> Command {
        let mut command = Command::new(&self.client);
        command
            .env_clear()
            .envs(&self.inherited)
            .current_dir(&self.cwd)
            .env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}"))
            .env("ANTHROPIC_CUSTOM_HEADERS", capability.child_header())
            .env("CLAUDE_CODE_SUBAGENT_MODEL", WORKER_MODEL)
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .env("CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK", "1")
            .args(["--model", PARENT_MODEL]);
        if let Some(id) = &self.resume {
            command.args(["--resume", id]);
        }
        command
    }
}

/// How long a terminated child may take to exit cleanly before its group is
/// killed; Claude restores terminal modes and saves the session on SIGTERM.
const CHILD_TERMINATION_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Experimental session supervisor used by the Linux launcher. The caller
/// must supply a router containing only fixed approved destinations. This accepts
/// no URL or capability via argv. Real-client TTY/signal/resume validation is pending.
pub async fn supervise(
    spec: &LaunchSpec,
    router: axum::Router,
    stop: impl std::future::Future<Output = i32>,
) -> anyhow::Result<i32> {
    #[cfg(unix)]
    unsafe {
        let foreground = libc::tcgetpgrp(libc::STDIN_FILENO);
        anyhow::ensure!(
            foreground < 0 || foreground == libc::getpgrp(),
            "launch requires the foreground terminal; background invocation refused"
        );
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let capability = Capability::generate();
    let router = super::security::protect(router, capability.clone());
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    // A dedicated runtime owns every accepted connection and upstream task.
    // Dropping only axum::serve would leave accepted connections running.
    let thread = std::thread::spawn(move || -> anyhow::Result<()> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let result = runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener)?;
            tokio::select! {
                result = axum::serve(listener, router).into_future() => result,
                _ = shutdown_rx => Ok(()),
            }
        });
        runtime.shutdown_timeout(std::time::Duration::from_millis(500));
        result.map_err(Into::into)
    });
    struct ServerGuard(
        Option<tokio::sync::oneshot::Sender<()>>,
        Option<std::thread::JoinHandle<anyhow::Result<()>>>,
    );
    impl Drop for ServerGuard {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
            if let Some(thread) = self.1.take() {
                let _ = thread.join();
            }
        }
    }
    let server = ServerGuard(Some(shutdown_tx), Some(thread));
    // Bound socket is ready before spawn; no discovery race or reuse of another listener.
    let mut command = spec.command(port, &capability);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command.spawn()?;
    let _terminal = TerminalGuard::foreground(child.id());
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            #[cfg(unix)]
            unsafe {
                // Only the process group created for this child. Descendants
                // that intentionally daemonize are outside the session contract.
                // SIGCONT lets a suspended group act on the termination request;
                // the leader gets a short grace period to restore the terminal
                // and save the session before the group is killed.
                libc::kill(-(self.0.id() as i32), libc::SIGTERM);
                libc::kill(-(self.0.id() as i32), libc::SIGCONT);
                let deadline = std::time::Instant::now() + CHILD_TERMINATION_GRACE;
                while std::time::Instant::now() < deadline
                    && peek_exit_code(&mut self.0).ok().flatten().is_none()
                {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            #[cfg(not(unix))]
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = ChildGuard(child);
    tokio::pin!(stop);
    let mut poll = tokio::time::interval(std::time::Duration::from_millis(20));
    loop {
        tokio::select! {
            code = &mut stop => return Ok(code),
            _ = poll.tick() => {
                if let Some(code) = peek_exit_code(&mut child.0)? { return Ok(code); }
                #[cfg(target_os = "linux")]
                if job_control::child_stopped(&child.0)? {
                    // Blocks while this job is suspended; the shell resumes it.
                    job_control::follow_child_stop(child.0.id() as i32);
                }
                anyhow::ensure!(!server.1.as_ref().unwrap().is_finished(), "local listener stopped; child terminated");
            }
        }
    }
}

/// Keep a terminated Linux leader unreaped until owned-group cleanup completes,
/// so its PID cannot be reused while signals are sent to that process group.
pub(crate) fn peek_exit_code(child: &mut std::process::Child) -> std::io::Result<Option<i32>> {
    #[cfg(target_os = "linux")]
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        if libc::waitid(
            libc::P_PID,
            child.id(),
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        ) != 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if info.si_pid() == 0 {
            return Ok(None);
        }
        Ok(Some(if info.si_code == libc::CLD_EXITED {
            info.si_status()
        } else {
            128 + info.si_status()
        }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(child.try_wait()?.map(exit_code))
    }
}

#[cfg(not(target_os = "linux"))]
fn exit_code(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
    }
    #[cfg(not(unix))]
    {
        status.code().unwrap_or(128)
    }
}

/// Runs `f` with SIGTTOU blocked on the calling thread only, so a terminal
/// ownership change from a background group neither stops the process nor
/// affects the server runtime's threads.
#[cfg(unix)]
fn with_sigttou_blocked<T>(f: impl FnOnce() -> T) -> T {
    unsafe {
        let mut block: libc::sigset_t = std::mem::zeroed();
        let mut previous: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut block);
        libc::sigaddset(&mut block, libc::SIGTTOU);
        libc::pthread_sigmask(libc::SIG_BLOCK, &block, &mut previous);
        let result = f();
        libc::pthread_sigmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
        result
    }
}

/// Gives the child's group the terminal and returns it on exit, but only while
/// the child still owns it: after `bg` the shell's group must keep it. The
/// terminal modes saved at launch are restored with it, so a child killed
/// before it could reset raw mode does not leave the shell's terminal broken.
#[cfg(unix)]
struct TerminalGuard(Option<(i32, i32, Option<libc::termios>)>);
#[cfg(not(unix))]
struct TerminalGuard;
impl TerminalGuard {
    fn foreground(child: u32) -> Self {
        #[cfg(unix)]
        unsafe {
            let previous = libc::tcgetpgrp(libc::STDIN_FILENO);
            if previous >= 0 {
                let mut modes: libc::termios = std::mem::zeroed();
                let modes = (libc::tcgetattr(libc::STDIN_FILENO, &mut modes) == 0).then_some(modes);
                let result =
                    with_sigttou_blocked(|| libc::tcsetpgrp(libc::STDIN_FILENO, child as i32));
                if result == 0 {
                    libc::kill(-(child as i32), libc::SIGCONT);
                    return Self(Some((previous, child as i32, modes)));
                }
            }
            Self(None)
        }
        #[cfg(not(unix))]
        {
            let _ = child;
            Self
        }
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some((previous, child, modes)) = self.0 {
            unsafe {
                if libc::tcgetpgrp(libc::STDIN_FILENO) == child {
                    with_sigttou_blocked(|| {
                        libc::tcsetpgrp(libc::STDIN_FILENO, previous);
                        if let Some(modes) = modes {
                            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &modes);
                        }
                    });
                }
            }
        }
    }
}

/// Mirrors the child's job state, so Ctrl-Z, `fg` and `bg` behave as they do
/// for a directly launched Claude: the child stops on its own group's
/// terminal signal, then the wrapper returns the terminal and stops its job.
#[cfg(target_os = "linux")]
mod job_control {
    use super::with_sigttou_blocked;

    /// Consumes one stop notification without reaping: no `WEXITED` and no
    /// `WNOWAIT`, so exit status stays available to `peek_exit_code`.
    pub(super) fn child_stopped(child: &std::process::Child) -> std::io::Result<bool> {
        unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            if libc::waitid(
                libc::P_PID,
                child.id(),
                &mut info,
                libc::WSTOPPED | libc::WCONTINUED | libc::WNOHANG,
            ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(info.si_pid() != 0 && info.si_code == libc::CLD_STOPPED)
        }
    }

    fn sigtstp_ignored() -> bool {
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            libc::sigaction(libc::SIGTSTP, std::ptr::null(), &mut action) == 0
                && action.sa_sigaction == libc::SIG_IGN
        }
    }

    /// Must run on the main thread: the kernel delivers the self-sent SIGTSTP
    /// to it, so `kill` returns only once the job has been continued. In an
    /// orphaned process group the kernel discards SIGTSTP and this returns
    /// immediately, resuming the child.
    pub(super) fn follow_child_stop(child_pgid: i32) {
        unsafe {
            if libc::isatty(libc::STDIN_FILENO) != 1 {
                return;
            }
            let wrapper_pgid = libc::getpgrp();
            if libc::tcgetpgrp(libc::STDIN_FILENO) == child_pgid {
                with_sigttou_blocked(|| libc::tcsetpgrp(libc::STDIN_FILENO, wrapper_pgid));
            }
            if !sigtstp_ignored() {
                libc::kill(0, libc::SIGTSTP);
            }
            // Continued: in the foreground (`fg`) the child gets the terminal
            // back; in the background (`bg`) the shell keeps it.
            if libc::tcgetpgrp(libc::STDIN_FILENO) == wrapper_pgid {
                with_sigttou_blocked(|| libc::tcsetpgrp(libc::STDIN_FILENO, child_pgid));
            }
            libc::kill(-child_pgid, libc::SIGCONT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_scoped_environment_and_resume_arguments() {
        let spec = LaunchSpec::new(
            Path::new("/usr/bin/claude"),
            Path::new("/tmp/project with spaces"),
            Some("session-123"),
            BTreeMap::from([("HOME".into(), "/tmp/isolated".into())]),
        )
        .unwrap();
        let cap = Capability::generate();
        let command = spec.command(12345, &cap);
        let env: BTreeMap<_, _> = command
            .get_envs()
            .map(|(k, v)| (k.to_str().unwrap(), v.unwrap().to_str().unwrap()))
            .collect();
        assert_eq!(env["CLAUDE_CODE_SUBAGENT_MODEL"], "gpt-6-astra[1m]");
        assert_eq!(env["ANTHROPIC_BASE_URL"], "http://127.0.0.1:12345");
        assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"));
        assert!(!env.contains_key("CLAUDE_CODE_SUBAGENT_MODEL_FORCE"));
        assert!(!env.contains_key("CCP_TRAFFIC_LOG"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["--model", "claude-opus-5-5", "--resume", "session-123"]
        );
        assert_eq!(
            command.get_current_dir().unwrap(),
            Path::new("/tmp/project with spaces")
        );
        assert!(
            LaunchSpec::new(
                Path::new("/bin/client"),
                Path::new("/tmp"),
                Some("--settings=bad"),
                BTreeMap::new()
            )
            .is_err()
        );
        let error = LaunchSpec::new(
            Path::new("/bin/client"),
            Path::new("/tmp"),
            None,
            BTreeMap::from([("ANTHROPIC_AUTH_TOKEN".into(), "CANARY".into())]),
        )
        .unwrap_err();
        assert!(!error.to_string().contains("CANARY"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_client_environment_exit_and_listener_cleanup() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let client = dir.path().join("fake-client");
        std::fs::write(&client, "#!/bin/sh\n[ \"$CLAUDE_CODE_SUBAGENT_MODEL\" = 'gpt-6-astra[1m]' ] || exit 90\n[ -z \"${ANTHROPIC_AUTH_TOKEN+x}\" ] || exit 91\nprintf '%s' \"$ANTHROPIC_BASE_URL\" > endpoint\nexit 17\n").unwrap();
        std::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o700)).unwrap();
        let spec = LaunchSpec::new(&client, dir.path(), None, BTreeMap::new()).unwrap();
        let code = supervise(&spec, axum::Router::new(), std::future::pending())
            .await
            .unwrap();
        assert_eq!(code, 17);
        tokio::task::yield_now().await;
        let endpoint = std::fs::read_to_string(dir.path().join("endpoint")).unwrap();
        let address = endpoint.trim_start_matches("http://");
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_reaps_owned_child() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let client = dir.path().join("fake-client");
        std::fs::write(&client, "#!/bin/sh\nexec /bin/sleep 60\n").unwrap();
        std::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o700)).unwrap();
        let spec = LaunchSpec::new(&client, dir.path(), None, BTreeMap::new()).unwrap();
        let start = std::time::Instant::now();
        let code = supervise(&spec, axum::Router::new(), async {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            130
        })
        .await
        .unwrap();
        assert_eq!(code, 130);
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn cancellation_stops_grandchild_and_preserves_signal_codes() {
        use std::os::unix::fs::PermissionsExt;
        for (signal, expected) in [("INT", 130), ("TERM", 143)] {
            let dir = tempfile::tempdir().unwrap();
            let client = dir.path().join("fake-client");
            std::fs::write(
                &client,
                format!("#!/bin/sh\n/bin/sleep 60 &\necho $! > grandchild\nkill -{signal} $$\n"),
            )
            .unwrap();
            std::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o700)).unwrap();
            let spec = LaunchSpec::new(&client, dir.path(), None, BTreeMap::new()).unwrap();
            let code = supervise(&spec, axum::Router::new(), std::future::pending())
                .await
                .unwrap();
            assert_eq!(code, expected);
            let pid = std::fs::read_to_string(dir.path().join("grandchild")).unwrap();
            let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid.trim()));
            // An orphan can briefly remain a zombie until init reaps it, but
            // must not remain executable after the session is gone.
            assert!(stat.is_err() || stat.unwrap().split_whitespace().nth(2) == Some("Z"));
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn cancellation_drops_active_request_work() {
        use std::{
            os::unix::fs::PermissionsExt,
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
            },
        };
        let dir = tempfile::tempdir().unwrap();
        let client = dir.path().join("fake-client");
        std::fs::write(&client, "#!/usr/bin/python3\nimport os,urllib.request\nk,v=os.environ['ANTHROPIC_CUSTOM_HEADERS'].split(': ',1)\nr=urllib.request.Request(os.environ['ANTHROPIC_BASE_URL']+'/v1/messages',data=b'{\"model\":\"opus\"}',headers={k:v})\nurllib.request.urlopen(r,timeout=10).read()\n").unwrap();
        std::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o700)).unwrap();
        let state = Arc::new(AtomicUsize::new(0));
        let observed = state.clone();
        struct Active(Arc<AtomicUsize>);
        impl Drop for Active {
            fn drop(&mut self) {
                self.0.store(2, Ordering::SeqCst);
            }
        }
        let router = axum::Router::new().route(
            "/v1/messages",
            axum::routing::post(move || {
                let observed = observed.clone();
                async move {
                    let _guard = Active(observed.clone());
                    observed.store(1, Ordering::SeqCst);
                    std::future::pending::<()>().await;
                    axum::http::StatusCode::OK
                }
            }),
        );
        let spec = LaunchSpec::new(&client, dir.path(), None, BTreeMap::new()).unwrap();
        let stop = async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while state.load(Ordering::SeqCst) != 1 {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            130
        };
        assert_eq!(supervise(&spec, router, stop).await.unwrap(), 130);
        assert_eq!(state.load(Ordering::SeqCst), 2);
    }
}
