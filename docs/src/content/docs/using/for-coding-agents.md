---
title: For coding agents
description: Work on this development fork without changing account settings or exposing credentials.
---

Use the [source-build quickstart](/getting-started/) and [development status](/reference/development-status/). Documentation is local and unhosted. A local build generates [the index](/llms.txt) and [assembled documentation](/llms-full.txt).

## Configuration contract

The proxy process needs `CCP_ALIAS_PROVIDER=anthropic` for dual mode. The built-in default remains `codex`. Client `ANTHROPIC_BASE_URL` selects the loopback proxy. Do not add dummy authentication values: `ANTHROPIC_AUTH_TOKEN`, API keys or an API-key helper can override Claude Code's subscription login. Settings-file environment entries may override shell values. See [authentication precedence](/using/configure-claude-code/).

Claude Code owns Anthropic login and refresh. The proxy owns a separate Codex login. Never copy or edit the native Codex CLI credential store to configure this proxy.

Inspect models without making inference calls:

~~~sh
CCP_ALIAS_PROVIDER=anthropic claude-code-proxy models
CCP_ALIAS_PROVIDER=anthropic claude-code-proxy models --full
~~~

Implementation and tests are authoritative: `src/config.rs`, `src/registry.rs`, `src/server.rs`, `src/providers/anthropic/` and `tests/`. Preserve original native fields, signed thinking, auth/beta/version headers, errors and streaming. Never fabricate reasoning signatures or forward incoming Anthropic credentials to translating providers.

## Verification and diagnostics

Use isolated configuration and loopback mocks. Formatting, Clippy, build and test commands are in [development status](/reference/development-status/). Live inference, account login, normal-environment installation and settings changes require explicit authorization beyond repository development.

The debug helper starts a local proxy and captures traffic; using its client instructions can make live requests. Run it only for an authorized investigation. Native errors stream through directly; do not assume every error has a saved payload. Other providers retain their diagnostic captures.

Prompts, tool inputs, file paths, captures and logs can contain private data even when known credential fields are redacted. Never publish credentials or unreviewed diagnostics. Keep the listener on loopback. Do not upload captures or alter external service settings without authorization.

## Confirmed parent/subagent setup

The user confirmed Opus 5.5 parent + `gpt-6-astra[1m]` general-purpose operation on Claude Code 2.1.299 after restarting through the same proxy-enabled launcher. See [setup, model precedence, context limits and verification](/using/subagents/). This is a limited real-use confirmation; broader live regression remains outstanding.
