---
title: Linux launcher development
description: Experimental Linux sessions, offline diagnostics and local artifact staging.
---

The distinct `claude-code-dual` entry point is an experimental Linux/WSL2 launcher. It coexists with the existing proxy executable. There is no published download installer. Keep your working installation while evaluating the separate candidate.

## Install availability

**Sol is outside this candidate's contract.** `claude-code-dual` fixes the worker default to `gpt-6-astra[1m]` and rejects `gpt-6.1-sol` requests. There is no Sol selection flag or supported environment override. The [named Sol agent recipe](/using/subagents/#select-sol-for-a-named-agent) applies to the existing `claude-code-proxy` workflow only; the user-reported mixed Astra/Sol success does not validate this launcher for Sol.

Build the feature branch in a separate checkout with `cargo build --bins --locked`. `target/debug/claude-code-dual contract` prints the model contract. `doctor --json` performs bounded offline checks and exits 2 while readiness is unverified. It does not execute Claude, hooks, helpers or MCP, make network requests, or read credential stores. Unknown remote/managed policy and client version stay unknown.

The candidate platform is Ubuntu 24.04/26.04 x86_64 with GNU libc >=2.39, including those distributions under WSL2. Only local WSL Ubuntu 26.04 testing has been performed. Native Windows, macOS, other distributions, ARM and musl are outside this scope.

## Authenticate separately

Continue to use Claude Code's own subscription login and the proxy's separate Codex login. These tools do not perform automatic login or credential migration. The existing manual [quickstart](/getting-started/) remains available for users who already chose that workflow; it is not an unauthenticated fallback from the new launcher's gate.

## Launch and verification

After the two separate logins, run `target/debug/claude-code-dual launch` from your project. It finds `$HOME/.local/bin/claude` or an absolute PATH entry; use `--claude /absolute/path` if needed. Resume with `launch --resume SESSION_ID`. The launcher requires Claude Code 2.1.299 and refuses detected auth/model overrides or managed configuration it cannot validate. Remove the obsolete `ANTHROPIC_AUTH_TOKEN=unused` from the launcher you control; never replace saved subscription OAuth with a dummy gateway credential.

Actual Claude 2.1.299 passed controlled, network-isolated tests for synthetic saved OAuth plus `x-claude-dual-capability`, Astra worker routing, an explicit Opus reviewer, resumed history, and preserved safety-hook/permission behavior. These tests did not call live providers or validate subscription entitlement. Remote managed policy remains unverified and is not bypassed.

Normal launch preserves project settings, hooks, permissions, MCP and the child's inherited environment. It passes only `--model claude-opus-5-5` and optional `--resume`; there are no safe-mode, settings-source or permission overrides. Auth/model conflicts fail explicitly. The proxy process separately uses fixed provider defaults with capture disabled, while retaining its existing separate Codex credential location. Project hooks and MCP can inherit the local capability: use a trusted project, and do not log the child environment.

The listener primitive generates a random capability, rejects missing/wrong/duplicate capabilities before dispatch, rejects browser Origin requests, restricts methods/routes/models, and strips the local header before handlers. Native forwarding strips it defensively too. Never put the capability in URLs, argv or logs. Same-user malware isolation is not promised.

The supervisor owns a loopback port-0 listener, cwd, validated resume identifiers, scoped capability and a child process group. It preserves foreground terminal input, returns child exit status, and terminates owned descendants and active request work on shutdown. Background terminal invocation is refused. Fake-client tests cover wrapper lifecycle behavior; complete real-client interactive UI certification and fallback behavior remain release gates. Intentionally daemonized descendants are outside this contract. No detached service is supported.

Parent: `claude-opus-5-5`. General-purpose default: `gpt-6-astra[1m]`, without FORCE. Literal native `opus` now resolves to `claude-opus-5-5`. Reviewer invocation and per-call overrides remain client responsibilities. The 1M setting is a client assumption, not verified upstream capacity. See [subagents](/using/subagents/).

## Local staging, manual upgrade and rollback

`scripts/dual-install.py` supports offline staging and an exact-version downloader. **No published assets exist yet.** It requires Python 3.10+, a dedicated absolute root, exact version, and a TAR.GZ with only both Linux x86_64 ELF binaries and LICENSE. It verifies an independently supplied SHA-256, bounds total decompression, rejects links/traversal, and never executes artifacts. A supplied hash does not authenticate a publisher unless its source is independently trusted.

Invocation shapes, with placeholders replaced only by reviewed local artifacts:

~~~text
python3 scripts/dual-install.py --root /absolute/dedicated/dual install-local --archive /absolute/candidate.tar.gz --version 0.1.45-dev.1 --sha256 <independently-verified-sha256>
python3 scripts/dual-install.py --root /absolute/dedicated/dual activate --version 0.1.45-dev.1
python3 scripts/dual-install.py --root /absolute/dedicated/dual uninstall
~~~

When an approved artifact and independently trusted digest exist, the download
command shape is `python3 scripts/dual-install.py --root /absolute/dedicated/dual
install-version --version EXACT_VERSION --sha256 TRUSTED_SHA256`. It fetches only
this repository's exact `vVERSION` Linux asset, rejects non-HTTPS/unapproved-host
redirects, sends no credentials, ignores ambient proxies, and enforces a 256 MiB
size limit plus a 60-second worker deadline (up to two seconds for cleanup).
Failures leave the existing installation unchanged. There is no `latest` lookup.

Bootstrap is still gated: an approved published version/artifact and digest
distribution tied to an independently established publisher identity are absent.
The installer itself must come from an audited/pinned checkout; it does not fetch
or execute another installer script. No curl-to-shell bootstrap, signing keys,
releases or account changes were created. Publishing and choosing a trust root
require a separate release decision; mocked downloader tests cannot establish it.

Upgrades add version directories; `activate` rolls back to a retained, hash-checked version. A parent-directory lock serializes mutations; activation uses an atomic pointer replacement. Existing versions are never overwritten. No sudo, Rust installation, shell-profile/PATH edits, credential copying or automatic updater is involved.

Uninstall checks all owned files first and refuses modifications, symlinks or unexpected entries. External credentials/data remain untouched. Interrupted operations can leave staging/activation files and cause a fail-closed refusal. Preserve and inspect those files before manual cleanup; do not recursively delete uncertain paths.

## Coexistence and release gates

Keep existing Homebrew packages and services unchanged. Use explicit executable paths. The eventual launcher must own a separate listener, not kill or silently adopt an existing one. No automatic migration is implemented.

Before a public download command can be populated, publish an approved version and establish authenticated provenance/digest distribution, installer bootstrap verification and platform CI. Candidate workflows must not create releases, signing keys or auth grants during this development task. The inherited LRU advisories remain unresolved; see [development status](/reference/development-status/).
