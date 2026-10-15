---
title: Monitor TUI
description: Use the claude-code-proxy monitor to inspect sessions, active and recent requests, providers, errors, token usage, API-equivalent cost, throughput, and setup.
---

`CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve` opens the monitor when stdout is an interactive terminal. The same process runs the HTTP listener.

To run the proxy as a service and attach the dashboard separately:

```sh
# Run this under your service manager, or leave it in another terminal.
CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve --no-monitor

# Attach from any terminal; repeat for additional dashboards.
claude-code-proxy monitor
```

Use `claude-code-proxy monitor --url http://127.0.0.1:19999` for a different port. Without `--url`, the port follows the usual proxy configuration. The attached dashboard reads the running service's existing history; it does not start a proxy or need provider credentials.

In an attached dashboard, `q` and `Ctrl-C` detach immediately and leave the service running. Multiple dashboards can attach independently. If the service becomes unavailable, the dashboard marks its last snapshot as stale and reconnects automatically. Network polling runs outside the terminal event loop.

## What the monitor shows

- Sessions grouped by Claude Code session ID and project
- Active request lifecycle and selected provider or model
- Recent requests, HTTP status, elapsed time, and errors
- Input, cache and output token totals
- API-equivalent cost of native Claude and Codex traffic (see below)
- Output throughput based on matched upstream timing and cumulative usage samples
- Paths to traffic captures when capture is enabled
- Configuration overrides and a ready-to-copy Claude Code setup

## Keyboard controls

| Key | Action |
| --- | --- |
| `Tab`, `←`, `→` | Change focused pane |
| `j`, `k`, `↓`, `↑` | Move selection |
| `Enter` | Open session or request details |
| `Esc` | Close details or an overlay |
| `?` | Toggle shortcut help |
| `b` | Toggle the setup overlay |
| `q` | Detach an attached dashboard; in the built-in dashboard, confirm proxy shutdown |
| `Ctrl-C` | Detach an attached dashboard; in the built-in dashboard, start shutdown (press again to force exit) |

The request table changes columns as the terminal width changes.

## API-equivalent cost

Subscriptions are not billed per token. The monitor shows what the observed
tokens would cost at the public API list prices, so you can compare the value of
each subscription and spot expensive requests. The figure is an estimate, not a
bill.

- **Where:** the header total (`≈API $12.34`), a `Cost` column in the Sessions
  and Recent tables on wide terminals, and the session and request detail views.
  Session detail breaks the total down by model; request detail shows uncached
  input, cache reads, 5-minute and 1-hour cache writes, the priced model and tier.
- **What is priced:** native Claude requests (`anthropic` provider) and Codex
  requests (`codex` provider) on the Messages, Responses and Chat Completions
  endpoints. Token counting requests and other providers show `-` and add
  nothing.
- **Prices:** hardcoded in `src/pricing.rs` from the
  [Claude](https://platform.claude.com/docs/en/about-claude/pricing) and
  [OpenAI](https://developers.openai.com/api/docs/pricing) pricing pages as of
  2026-10-14 (shown as "prices as of" in request detail). Codex `-fast`
  (priority), `-ultrafast` and `flex` tiers and Claude fast mode use their own
  rows. Long-context prices apply to the whole request above 272K prompt tokens
  (OpenAI) or 100K (Claude Haiku 5.5); prompt tokens include cache reads and
  writes.
- **Markers:** `~` means the cost includes locally counted tokens (a Codex input
  estimate that upstream never corrected, the standalone search route, or a
  Claude server-side fallback that bills several attempts). `n/a` means no
  published price applies, for example `gpt-5.3-codex-spark` or an unknown
  model; `?` means the response could not be observed. The header and session
  totals count these as `+N unpriced`.

Limitations: Codex server-side compaction calls are not metered; Codex does not
report cache writes, so they are priced as plain input; data residency, batch,
server-tool fees and refusal or fallback credits are ignored. A dashboard
attached to an older proxy shows no cost figures.

## Plain logs

Use plain output when the process runs under a service manager, in CI, or through a pipe:

```sh
CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve --no-monitor
```

Non-terminal stdout also selects plain mode. `CCP_LOG_STDERR=1` mirrors JSONL log events to stderr in plain mode.

Plain mode retains monitor accounting even with no dashboard attached. On Unix, SIGTERM starts graceful proxy shutdown; Ctrl-C does the same. The service manager owns the process lifetime.

## Demo mode

Explore the full interface without binding a port or using provider credentials:

```sh
claude-code-proxy demo
```

The deterministic simulation covers active, successful, and failed requests across providers, projects, throughput states, and responsive layouts.

## Background service

This fork has no Homebrew formula or packaged service. Run the source-built binary with `CCP_ALIAS_PROVIDER=anthropic` and `serve --no-monitor` in a dedicated terminal, or configure your own service manager explicitly. Its environment must preserve dual mode. The service manager determines stdout/stderr log paths; structured `proxy.log` uses the state directory. Claude authentication remains owned by the client.

Run `claude-code-proxy monitor` to inspect that service. The monitor endpoint only accepts loopback connections, even if the inference listener binds a LAN address. For a service on another machine, forward its port with SSH and point `monitor --url` at the local end of the tunnel.

History remains in the proxy's memory and resets when the proxy restarts. The dashboard polls snapshots every 250 ms and displays server-computed durations and throughput. The attached setup overlay describes its connection; provider setup remains with the service and its built-in dashboard.
