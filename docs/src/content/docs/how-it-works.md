---
title: How it works
description: Native Claude passthrough and separate Codex authentication in one session.
---

Start the proxy with `CCP_ALIAS_PROVIDER=anthropic`. Claude Code sends Messages requests to the loopback listener. Each request's model chooses a route.

## Native Claude route

Real `claude-*` IDs and the built-in Claude aliases select the fixed `https://api.anthropic.com` upstream. Native selection takes precedence over session affinity and auto-review overrides. Claude Code supplies and refreshes its saved subscription OAuth login; the proxy forwards the original authentication, version and beta headers. It does not read or store Anthropic credentials.

Original request fields are preserved. Local aliases and context suffixes are resolved before forwarding. Native signed thinking is preserved; foreign visible reasoning is converted to text without inventing signatures. Responses retain native status codes, error bodies and streamed bytes. Redirects are disabled. Client cancellation closes the upstream stream.

Native `/v1/messages/count_tokens` requests go to Anthropic with the same authentication boundary. They are not local estimates.

## Codex and other translating routes

Explicit GPT IDs select Codex using the proxy's separate Codex OAuth login. Incoming client headers and query credentials are discarded before translating-provider dispatch, so Claude credentials do not become Codex credentials. Other providers retain their existing independent login or configured API-key mechanisms.

Translators map messages, tools and visible history into each provider's protocol and reconstruct Anthropic response events. Codex's own encrypted reasoning can replay on Codex turns; native Anthropic signatures and redacted thinking are not Codex reasoning. Translating providers use local token estimates rather than Anthropic's count endpoint.

## Sessions and API surfaces

Switch with `/model gpt-6-sol` and `/model claude-opus-5` after a completed turn. A GPT turn does not redirect a subsequent Claude alias away from Anthropic in dual mode. Unfinished tool exchanges can still be rejected by the receiving provider.

Session IDs support monitor grouping and provider continuity. Cursor conversation state and optional Codex response/compaction state remain in memory; restarting clears that state. Claude Code's conversation history remains available.

Optional OpenAI-compatible endpoints serve translating providers. Native Anthropic passthrough is available only through the Messages and count-token endpoints. Unknown non-Claude models return a routing error; explicit new `claude-*` IDs can pass through without a catalog update.

See [authentication](/using/configure-claude-code/), [HTTP contracts](/reference/http-api/) and [development limitations](/reference/development-status/). This architecture is mock-tested; live subscription switching remains unverified.
