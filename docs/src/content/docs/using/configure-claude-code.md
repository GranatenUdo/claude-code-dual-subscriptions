---
title: Configure Claude Code
description: Preserve Claude Code subscription OAuth while routing GPT requests through separate Codex authentication.
---

Configure the server and client separately. For this fork's primary workflow,
start the server with `CCP_ALIAS_PROVIDER=anthropic`. The built-in default
remains `codex` for compatibility.

## Client launch for dual subscriptions

Keep Claude Code's saved claude.ai login and set the base URL:

```sh
env -u ANTHROPIC_AUTH_TOKEN -u ANTHROPIC_API_KEY \
  ANTHROPIC_BASE_URL=http://127.0.0.1:18765 \
  CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
  CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK=1 \
  claude --model claude-opus-5
```

These are manual live-use instructions. This fork has only been mock-tested.

## Authentication precedence

Setting only `ANTHROPIC_BASE_URL` retains Claude Code's saved subscription login.
An `ANTHROPIC_AUTH_TOKEN`, approved `ANTHROPIC_API_KEY`, or configured
`apiKeyHelper` takes precedence over that login. A dummy token disables the
subscription path and can cause a native Anthropic 401.

Settings-file `env` entries can override shell values. Unsetting shell variables
does not remove settings-file credentials or helpers. Inspect existing
configuration and choose the intended credential source without printing or
copying secrets. Do not log out merely to use this proxy.
See [Claude Code auth precedence](https://code.claude.com/docs/en/llm-gateway-connect#conflicts-with-an-existing-login).

The proxy forwards Anthropic auth, OAuth beta and version headers only to the
fixed Anthropic upstream. It never reads or refreshes Anthropic credentials.
Codex uses `claude-code-proxy codex auth login` independently.
Deliberately supplied Anthropic API keys are forwarded too, but are API billing,
not the subscription workflow.

## Model and context controls

| Client setting | Purpose |
| --- | --- |
| `ANTHROPIC_BASE_URL` | Local proxy URL |
| `ANTHROPIC_MODEL` or `--model` | Initial model; use `/model` for subsequent turns |
| `ANTHROPIC_SMALL_FAST_MODEL` | Optional fixed background model, independent of the main model |
| `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` | Reduces nonessential client traffic |
| `CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK=1` | Avoids fallback replay after a partially completed stream |

A fixed small model keeps using its provider even when the main model changes.
Leave it unset initially or choose a concrete ID intentionally.

The native Anthropic route preserves the requested streaming mode. Translating
providers can accumulate upstream streams into non-streaming responses.
A trailing `[1m]` is a client context hint stripped by the proxy; it does not
increase account context limits. Use a conservative limit valid for every model
you intend to switch between, or omit overrides and compact before switching a
large history. No single context override is prescribed for dual mode.

## Scope and persistence

Start with a process-scoped launch. Persistent settings are optional and affect
other Claude Code sessions; this project does not change them automatically.
If you elect to persist routing, keep only the intended client variables and
remove conflicting credential overrides yourself. Restart the client after
changing its launch environment.

`CCP_*`, `PORT` and proxy `config.json` apply to the server.
For example, `{"aliasProvider":"anthropic"}` in the proxy configuration enables
dual mode persistently. See [Configuration](/reference/configuration/).