# Controlled Linux client evidence

Historical record (2026-10-14, Claude Code 2.1.299; the launcher has since been
re-pinned to 2.1.300, see below). WSL2 Ubuntu 26.04, x86_64/glibc 2.43. Actual client:
`/home/renade/.local/bin/claude` → `/home/renade/.local/share/claude/versions/2.1.299`.
An isolated `--version` returned `2.1.299 (Claude Code)`.

The reproducible probe is `scripts/probe-claude-contract.py`. All credentials in
the probe are synthetic, generated in an ephemeral home. No real home, config,
keychain, credentials, project or host network is mounted. The server and client
share only the namespace loopback. This is transport/client-behavior evidence,
not live subscription acceptance or a million-token capacity test.

Run from Linux with the reviewed client and script paths substituted:

```sh
bwrap --unshare-all --die-with-parent --new-session \
  --ro-bind /usr /usr --symlink usr/bin /bin \
  --ro-bind /lib /lib --ro-bind /lib64 /lib64 \
  --ro-bind /absolute/claude-2.1.299 /client \
  --ro-bind "$PWD/scripts/probe-claude-contract.py" /probe.py \
  --proc /proc --dev /dev --tmpfs /tmp --dir /home/test \
  --chdir /home/test --clearenv --setenv HOME /home/test \
  --setenv PATH /usr/bin:/bin /usr/bin/python3 /probe.py auth
```

Other scenarios: `general-purpose`, `reviewer`, `resume`, `policy`.
The script reports allowlisted metadata, not request bodies or credential values.

Observed successful checks:

- `auth`: two requests reached the mock, both with the exact synthetic saved
  OAuth token, OAuth beta and separate capability; no API key. Native mock error
  reached the client. Neither secret occurred in captured output.
- `general-purpose`: parent `claude-opus-5-5`; model-less Agent invocation used
  `gpt-6-astra` with the 1M beta hint; parent stayed Opus. No FORCE was set.
- `reviewer`: explicit per-call `model: opus` selected `claude-opus-5-5` despite
  the Astra environment default. This verifies the override, not whether a real
  parent will choose to invoke a reviewer.
- `resume`: both invocations exited 0; the resumed request contained the prior
  synthetic assistant text and retained OAuth/capability headers.
- `policy`: a separate project's `.claude/settings.json` ran a synthetic UserPromptSubmit safety hook;
  a settings-denied Bash tool was unavailable and its sentinel was not created.
  The hook sentinel existed. No safety bypass or real configuration was involved.

Re-pin to 2.1.300 (2026-10-15): the same `probe-claude-contract.py all` command,
run against a pinned copy of `~/.local/share/claude/versions/2.1.300` (sha256
`674deba0bba9aee479f4503c438b49303b6fdd304b9a471b14977aaec5215453`) with
`--setenv DISABLE_AUTOUPDATER 1`, passed all five scenarios with the same
observations as above: saved OAuth, OAuth beta and capability on every request;
Astra worker with the 1M hint; explicit Opus reviewer; resumed history; safety
hook ran and the denied Bash sentinel was absent; no secret in output. The
launcher contract was changed only after this passed.

Early subagent attempts triggered Claude's auto-mode classifier and did not prove
worker routing. The final synthetic Agent scenarios explicitly allow only Agent
in `dontAsk` mode. These testing flags are not production launcher flags.

The launcher preserves normal project hooks, permissions, MCP and child
environment. It uses only `--model` and optional `--resume`; it does not pass
safe-mode, setting-source exclusions or permission overrides. Synthetic isolation
comes from bwrap, not from weakening the user's normal launch policy.

Fake-client lifecycle checks cover child exit, descendant group cleanup, active
request cancellation, foreground TTY input, Ctrl-C, and wrapper signal status.
They do not prove every interactive behavior of the real Claude UI. Deliberately
daemonized descendants and remote managed policy are outside certification.

The corrected `scripts/probe-claude-tty.py all` uses the same isolated mounts plus
the built launcher at `/launcher`, this script at `/terminal_probe.py`, and the
contract probe at `/probe.py`. It submits no model prompt. All three cases passed:
direct Claude trust refusal exits1, launcher trust refusal also exits1, and
launcher SIGTERM exits143. The real client's normal trust gate was visible and
keyboard input declined it; no credential values appeared in output.

The original assertion was a harness defect: it matched raw bytes despite ANSI
cursor positioning splitting words, used a zero-sized PTY, sent input during
initial rendering, and assumed trust refusal exited0. The fixed harness uses
40x120 geometry, normalized screen text, a one-second render/input readiness
delay, and verifies the refusal code against direct Claude. No launcher defect
was demonstrated and no safety gate was bypassed. This establishes startup,
trust-gate input and termination only; complete interactive conversations and
Ctrl-Z/suspend/resume job control remain unverified. Do not name the script
`/tty.py`, which shadows Python's standard module.
