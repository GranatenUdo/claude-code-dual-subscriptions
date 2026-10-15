# Linux dual-session implementation plan

Goal: a distinct Linux/WSL2 command with offline diagnostics and a capability-protected session, without changing existing installations or Claude settings.

Architecture: the new binary owns a loopback port-0 listener and child lifecycle. A separate local header authenticates requests before any routing or logging; it is removed before dispatch. Actual Linux Claude 2.1.299 has proved synthetic saved-OAuth/header compatibility; launch is now wired with explicit preflight checks. Normal project settings and guardrails are preserved. Installer operations own only a dedicated version directory and activation pointer.

Scope: Linux x86_64 GNU; Ubuntu 24.04/26.04 and WSL2 running those distributions are candidates, not certified release platforms. No native Windows/macOS, automatic updater, daemon, role presets, credential migration, releases, or live calls. No FORCE. Same-user malware isolation is not promised.

## Gates and evidence

- [x] Branch: feature/linux-dual-launcher; baseline 9769c17.
- [x] Checkout has no AGENTS.md/.agents instructions. checkle.toml runs fmt, strict Clippy, build and tests.
- [x] Explicit discovery as WSL user renade found `/home/renade/.local/bin/claude` pointing to version 2.1.299. The earlier default-user PATH lookup was insufficient. Ubuntu-26.04 is x86_64, glibc 2.43.
- [x] Actual Linux Claude 2.1.299 in a bwrap network/filesystem namespace sent synthetic saved OAuth, OAuth beta and the separate local capability together to the loopback mock; no API key, no secret in captured output. No real credentials inspected. See `linux-client-evidence.md`.
- [x] Actual-client synthetic model precedence, explicit reviewer, resume and local safety-hook/denied-tool checks passed. Corrected terminal probes established trust-refusal input and SIGTERM status. See `linux-client-evidence.md` for the isolation and limits.
- [x] Re-pinned to Claude 2.1.300 after the unchanged synthetic contract probe passed on a pinned copy.
- [x] Actual-client certification through the real launcher preflight and supervisor (`scripts/certify.py`, test-only harness `tests/dual_certification.rs`): interactive conversations, tools, denied tool and safety hook, MCP, Astra worker, Opus reviewer, resume, Ctrl-C/SIGTERM/exit compared with direct Claude, terminal restoration, background-start refusal, three Ctrl-Z/`fg` cycles, owned-group cleanup and active-request cancellation, synthetic 401/429/5xx/interrupted/unsupported-model errors, and configuration refusals. See `DEVELOPMENT.md`.
- [ ] Remote managed policy remains uncertified; intentionally detached descendants remain outside the contract.
- [ ] BLOCKED: published artifacts/trust bootstrap; no releases exist. A user-supplied local artifact/hash is a developer staging mechanism, not a verified release channel.

## Tasks and acceptance

1. Model contract: parent claude-opus-5-5, worker gpt-6-astra[1m], explicit reviewer opus. Update/test literal opus mapping to claude-opus-5-5. Do not promise the parent invokes a reviewer. Client 1M assumption is distinct from upstream capacity.
2. Auth boundary: create src/dual/security.rs; random capability, redacted Debug, duplicate/missing/wrong-header rejection before routing, remove capability before handlers, restrict route/method, no URL capability. Synthetic HTTP tests assert zero handler/upstream calls on rejection.
3. Offline doctor: create src/bin/claude-code-dual.rs and src/dual/doctor.rs; no child execution, helpers, hooks, MCP, network or credential files. Bounded regular-file config inspection with symlink/oversize/malformed/unknown-managed-policy failures, key-only redacted output. Test hostile values and canaries.
4. Lifecycle: create src/dual/session.rs; port 0, exact scoped child environment, no shell interpolation, cwd and arguments preserved, capture off, interrupt/crash cleanup, no attach to existing listeners. Fake-client tests establish wrapper behavior only. Launch now runs after explicit configuration/client preflight; failed preflight starts no listener.
5. Installer: create scripts/dual-install.py plus isolated tests; exact local version/hash, reject archives with links/traversal/extra members, version-owned manifest, atomic activation, manual rollback, owned-file uninstall retaining external credentials. Exact-version downloading is implemented and mock-tested with a mandatory independent hash and bounded worker; real downloads remain gated on published assets and release trust. No sudo/PATH profile edits/brew changes.
6. Release preparation: least-permission Linux-only draft/manual workflow; no grants, secrets or signing keys created. Artifact provenance is necessary before enabling a download installer. No publishing action in this task.
7. Journey docs: install availability -> separate authentication -> preflight and launch -> verification; coexistence/recovery/uninstall. Preserve removed promotion/provider docs and unresolved LRU caveat.
8. Verification: focused tests, full locked fmt/Clippy/build/test, installer tests, docs build/links; independent code review; report passed/failed/not-run separately.

## Review focus

- Conflicting project/managed configuration must not silently replace the capability, destination, model, or auth source.
- Browser-origin and unauthenticated requests must never reach providers; capability must never appear upstream or in logs.
- Signal/child failure must not leave a listener alive or kill unrelated processes.
- Uninstall/update must refuse unowned, modified or symlinked paths.
- Fake-client tests must never be described as real Claude OAuth/TTY/resume validation.

## Progress

Core implementation and recorded local verification are complete; remaining
certification and distribution gates are listed above. Actual-client synthetic auth,
model selection, reviewer, resume and safety-hook/permissions checks passed.
Real launch is wired; normal hooks/settings/MCP are preserved. Independent review
identified and drove fixes for environment redaction, installer locking and
decompression, process-group cleanup, active-request cancellation, background
terminal ownership, wrapper signal codes and settings conflict checks.
Full locked Rust suite passed: 1,150 tests, zero failures. Four fake-client PTY
tests and 16 installer/downloader tests passed. Five actual-client synthetic scenarios
passed. Strict Clippy passed; docs built 25 pages with 1,785 checked links/assets.
The original actual-client terminal UI-marker failure was a harness defect.
Three corrected cases passed: native and launcher trust refusal exit1, plus
launcher SIGTERM143; full conversation/job-control behavior remains a release gate.
Exact-version downloading now uses a mandatory independently trusted SHA-256 and
a bounded worker. Published assets and authenticated bootstrap data remain absent.
Final locked build/format results are recorded in DEVELOPMENT.md. No live providers, install, merge,
push, release or normal-environment changes have occurred on this feature branch.
