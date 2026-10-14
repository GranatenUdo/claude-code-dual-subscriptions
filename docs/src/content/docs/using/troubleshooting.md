---
title: Troubleshooting
description: Diagnose listener, authentication, model routing, streaming, context, Cursor bundle, rate-limit, logging, and traffic-capture problems.
---

## Proxy is unreachable

Check the process and liveness route:

```sh
curl http://127.0.0.1:18765/healthz
```

A healthy listener returns `{"ok":true}`. Confirm `ANTHROPIC_BASE_URL` uses the same address and port. `CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve --port 11435` and `PORT=11435` change the listener port. `--port` wins for that command.

## Native Claude 401 or unexpected Codex routing

Confirm the server was started with `CCP_ALIAS_PROVIDER=anthropic`, or equivalent proxy config. Restart it after changing this. Check Claude Code's login/status without printing credentials. Dummy `ANTHROPIC_AUTH_TOKEN`, approved API keys and `apiKeyHelper` override subscription OAuth; settings-file values can override shell unsets. Claude Code owns login and refresh. There is no proxy Anthropic login command.

A native Claude alias stays Anthropic after a GPT turn and bypasses auto-review overrides. If it does not, verify you are running this fork's binary rather than an upstream/Homebrew installation.

## Other-provider authentication error

Check the provider selected by the model, then inspect its credential:

```sh
claude-code-proxy codex auth status
claude-code-proxy kimi auth status
claude-code-proxy cursor auth status
```

Use that provider's login command when credentials are missing or expired. Codex requires ChatGPT subscription auth, not an OpenAI API key. These translating providers use proxy-owned credentials; Anthropic uses client-supplied auth.
OpenCode Go has no `auth status` command; configure its API key through
`CCP_OPENCODE_API_KEY`, `OPENCODE_API_KEY`, or `opencode.apiKey` in
`config.json`.

## Model returns HTTP 400

Run:

```sh
claude-code-proxy models
claude-code-proxy models --full
```

An unknown local ID returns a catalog in the error. A known ID can still be rejected upstream when the account, subscription, or region lacks access. Cursor's prefixed form, such as `cursor:gpt-5.5`, forces Cursor routing and avoids cross-provider collisions.

## Background requests fail

Set `ANTHROPIC_SMALL_FAST_MODEL` to a concrete routable ID. Claude Code sends title and small background tasks through that model independently of the main model.

## A tool runs twice

Set `CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK=1` for Claude Code. Retrying a partially completed stream as non-streaming can duplicate tool calls.

## Session reaches context limits

`[1m]` changes Claude Code's local compaction threshold and does not enlarge upstream context. Use a safe `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, remove `[1m]`, or compact earlier. A value tuned to one provider is not necessarily safe after a model switch. Compact at a completed turn boundary before moving a large history.

## Codex WebSocket fails

For a non-TUN local HTTP proxy, set both destination-scheme variables before starting the proxy:

| Variable | Value |
| --- | --- |
| `HTTP_PROXY` | `http://127.0.0.1:7890` |
| `HTTPS_PROXY` | `http://127.0.0.1:7890` |

Set them through the operating system, service manager, or shell, then start `CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve` in the same environment.

The default `wss://chatgpt.com` connection uses `HTTPS_PROXY`; a working proxy should show `CONNECT chatgpt.com:443`. The `http://` value is normal: it describes how to reach the proxy, while `HTTPS_PROXY` describes which destinations use it. Restart claude-code-proxy after changing these variables because the client and pooled WebSocket route are created at startup.

Check `NO_PROXY` when the proxy sees no request. Proxy connection, authentication, or CONNECT failure is returned as an error and never retried directly. Environment variables are supported; OS proxy settings and PAC/WPAD discovery are not automatic.

Use HTTP SSE to isolate transport behavior:

```sh
CCP_CODEX_TRANSPORT=http CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve
```

`auto` falls back only when WebSocket setup fails before sending the request. It does not replay an in-flight request.

## Cursor bundle cannot be found

Cursor needs the installed Cursor Agent JavaScript bundle for protobuf schemas. Confirm Cursor Agent is installed, then set:

```sh
CCP_CURSOR_AGENT_BUNDLE=/path/to/cursor-agent/index.js \
  CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve
```

## Rate limited

Upstream limits are shared with other clients on the same account. Codex limit responses and Kimi HTTP 429 responses surface as HTTP 429 with `retry-after`. Wait for the indicated interval or reduce concurrent traffic.

## Find the complete error

The monitor shows request detail. `proxy.log` contains structured JSONL events, and `errors/` stores captured translating-provider failures. Native Anthropic errors are streamed to the client; their bodies are not buffered into that capture path. A `request_failed` event includes the error file path.

For a source checkout, run the isolated helper:

```sh
./scripts/debug-proxy
```

It enables native alias routing, preserves the client login by unsetting shell gateway credentials, and prints a random local URL, a sourceable `client.env`, and artifact paths. It still reads the normal proxy credential directory and can make live calls when the client sends requests; it is not a mock-only test. Keep the terminal open, reproduce once, then press Ctrl-C. Captures contain sensitive prompt and tool content.

## Focused traffic capture

```sh
CCP_LOG_VERBOSE=1 \
CCP_TRAFFIC_LOG=1 \
  CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve --no-monitor
```

Traffic capture writes the inbound request, translated upstream request, upstream headers and events, and downstream events in emission order. Known credentials are redacted, but prompts and tool content are preserved. Disable capture and delete artifacts after diagnosis.

## Astra subagent still runs on Opus

On Claude Code 2.1.299, use `CLAUDE_CODE_SUBAGENT_MODEL=gpt-6-astra[1m]`, restart through the same proxy-enabled launcher, and omit the general-purpose Agent model argument. Full IDs in that argument failed its alias enum in the observed setup. Check definition/per-call overrides, FORCE, allowlists and fallback warnings. Verify the recorded provider/model in the monitor and transcript; see [the full procedure and reversal](/using/subagents/).

## Context is smaller than expected

Keep `[1m]` in the client Astra setting from startup. The monitor showing `gpt-6-astra` is expected because the proxy strips the suffix. Review existing compaction overrides and 1M-disable settings using [the context guide](/using/models-and-routing/#the-1m-hint). A tiny successful probe does not test a million-token prompt.
