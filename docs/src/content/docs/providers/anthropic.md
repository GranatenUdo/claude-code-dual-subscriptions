---
title: Anthropic
description: Keep Claude models on Anthropic while switching other models through the proxy in the same Claude Code session.
---

Anthropic passthrough is opt-in. Set `aliasProvider` to `anthropic` in `config.json`, or start the proxy with:

```sh
CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve
```

This sends `claude-*` models and the built-in `haiku`, `sonnet`, `opus`, and `fable` aliases to `api.anthropic.com`. Explicit GPT, Kimi, OpenCode Go and Cursor IDs retain their provider. A preceding GPT or Kimi turn does not change where the Claude aliases go. Restart the proxy after changing `aliasProvider`.

## Authentication and model switching

Sign in using Claude Code. The proxy forwards the credentials Claude Code includes in each request; it does not read the keychain, store Anthropic credentials, or refresh them. Codex continues to use its own `claude-code-proxy codex auth login`.

Launch Claude Code using its existing subscription login:

```sh
env -u ANTHROPIC_AUTH_TOKEN -u ANTHROPIC_API_KEY \
  ANTHROPIC_BASE_URL=http://127.0.0.1:18765 \
  CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
  claude --model claude-opus-5-5
```

Also remove dummy credentials from Claude Code settings if configured there. `ANTHROPIC_AUTH_TOKEN=unused` replaces the subscription login and causes the Anthropic route to fail. Deliberately configured Anthropic API credentials are forwarded too.

Setting only `ANTHROPIC_BASE_URL` retains the saved subscription OAuth login.
Gateway credentials (`ANTHROPIC_AUTH_TOKEN`, an approved `ANTHROPIC_API_KEY`, or
`apiKeyHelper`) take precedence over that login. Settings-file environment values
can override shell values, so unsetting a shell variable alone may not suffice.
See [Claude Code's authentication precedence](https://code.claude.com/docs/en/llm-gateway-connect#conflicts-with-an-existing-login).
The proxy preserves the OAuth `anthropic-beta` header. Claude Code remains
responsible for login and refresh. No shared Codex CLI credential file is modified.

This is a development fork tested with local mock servers. The user confirmed the limited Opus/Astra subagent setup below; broader live account behavior remains unverified. Non-Claude model routing is not officially supported by
Anthropic. Do not infer provider approval from successful local protocol tests.

Use `/model gpt-6-astra` and `/model claude-opus-5` to switch between the providers. Availability still depends on the selected provider account. The model catalog advertises known aliases, but explicit new `claude-*` IDs can be forwarded without a proxy release.

## Thinking and conversation history

Native Anthropic thinking and `redacted_thinking` blocks remain unchanged, including signatures. Requests with no local model rewrite or foreign thinking conversion retain their original bytes, including unknown fields, cache controls and beta headers.

When switching from GPT to Claude, unsigned thinking and thinking with proxy-owned `ccp:` signatures become `<previous_reasoning>` text blocks containing only the visible summary. When switching from Claude to GPT, visible thinking summaries similarly become text; opaque Claude signatures and redacted content are not sent as Codex reasoning. Valid native Codex reasoning continues to replay in its encrypted form on Codex turns. Responses are streamed unchanged, so the conversation keeps the original provider blocks for a later switch back.

This carries visible summaries across providers, not encrypted internal reasoning. Switch after a completed turn. Changing providers during an unfinished tool-use turn can still fail the receiving provider's history validation.

## HTTP behavior and monitor

`POST /v1/messages` and `POST /v1/messages/count_tokens` preserve query parameters and forward Anthropic authentication and beta headers. Local aliases and the `[1m]` suffix are resolved before forwarding. Redirects are not followed. Upstream status codes, request IDs, retry headers, response bodies and SSE events are preserved; connection-specific headers are removed.

Anthropic requests appear in the existing monitor, including usage and streamed errors. The provider does not expose Anthropic through the proxy's OpenAI-compatible endpoints. With `aliasProvider` set to `codex` (the default) or `kimi`, routing and session affinity keep their existing behavior.

Native Claude requests also bypass `CCP_AUTO_REVIEW_MODEL` overrides. Client
headers and query credentials are discarded before dispatch to translating
providers. Anthropic's production destination is fixed to `https://api.anthropic.com`;
the injected client constructor is for mock testing, not a runtime URL setting.
The listener remains loopback by default. Do not expose it to untrusted clients.

## Confirmed parent/subagent setup

The user confirmed Opus 5.5 parent + `gpt-6-astra[1m]` general-purpose operation on Claude Code 2.1.299 after restarting through the same proxy-enabled launcher. See [setup, model precedence, context limits and verification](/using/subagents/). This is a limited real-use confirmation; broader live regression remains outstanding.
