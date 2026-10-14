---
title: Dual subscriptions for Claude Code
description: Development documentation for native Claude OAuth passthrough alongside separate ChatGPT Codex OAuth.
---

**claude-code-dual-subscriptions** is a development fork of
[raine/claude-code-proxy](https://github.com/raine/claude-code-proxy).
The executable remains `claude-code-proxy`.

One local listener can route native Claude requests with the credentials supplied
by Claude Code, and GPT requests with the proxy's separate Codex subscription login.
Enable `CCP_ALIAS_PROVIDER=anthropic` before starting the proxy, then select models
in a single Claude Code session.

## Development status

The implementation passed 1,136 local tests, including eight Anthropic mock
integration tests, plus formatting, strict lint and a Linux/WSL build.
**Opus 5.5 parent + Astra subagent operation is user-confirmed on 2.1.299; broader live switching is unverified.** Non-Claude routing is not
officially supported by Anthropic. The inherited `lru 0.12.5` advisories remain
unresolved; see [limitations](/reference/compatibility-and-limitations/).

No fork release binaries or hosted docs site exist. These pages are repository
source and a local preview. Follow [Getting started](/getting-started/) to build
the actual fork from source rather than installing upstream.

## Authentication and routing

- Claude Code owns Anthropic login and refresh. The proxy preserves its OAuth,
  beta and version headers and native response bytes.
- Codex uses the proxy's own ChatGPT OAuth store. Anthropic credentials are
  discarded before dispatch to translating providers.
- Native Claude model selection takes priority over session affinity and
  auto-review overrides.
- Native signed thinking stays intact. Visible foreign reasoning becomes text;
  opaque reasoning is not transferable across providers.
- Kimi, OpenCode Go and Cursor guides remain available as inherited features.

Continue with [configuration](/using/configure-claude-code/),
[model switching](/using/switching-models-and-backends/), or
[request flow](/how-it-works/).
## Confirmed parent/subagent setup

The user confirmed Opus 5.5 parent + `gpt-6-astra[1m]` general-purpose operation on Claude Code 2.1.299 after restarting through the same proxy-enabled launcher. See [setup, model precedence, context limits and verification](/using/subagents/). This is a limited real-use confirmation; broader live regression remains outstanding.
