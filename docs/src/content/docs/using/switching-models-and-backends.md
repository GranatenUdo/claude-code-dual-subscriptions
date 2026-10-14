---
title: Switching models and backends
description: Switch native Claude and Codex models in one session while retaining separate subscription credentials.
---

The primary workflow keeps the client pointed at one local proxy. Changing a
model can change the provider within that session; changing the base URL requires
a new Claude Code process. Live switching is unverified; local tests exercise
routing, authentication isolation and mixed history.

## Prepare once per proxy process

```sh
CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve
```

Claude Code must already have its own subscription login, and the proxy must have
its separate Codex login. Use the [client launch](/using/configure-claude-code/)
without a dummy auth token. Neither login replaces the other.

## In-session switching

After the current response and all tool results are complete:

```text
/model gpt-6-sol
/model claude-opus-5
/model sonnet
```

With `aliasProvider=anthropic`, `sonnet` maps to `claude-sonnet-5`,
`opus` to `claude-opus-5`, `haiku` to `claude-haiku-4-5`, and `fable`
to `claude-fable-5`. Prefer explicit IDs when a different native version is
required. Account access is separate from local registration.

Claude aliases stay native even after a Codex or Kimi turn. Native Claude requests
also bypass the proxy's auto-review model override. GPT models continue to use
Codex. A fixed `ANTHROPIC_SMALL_FAST_MODEL` remains independent of these switches.

## Thinking and history

The conversation remains in Claude Code. Native signed and redacted Anthropic
thinking blocks retain their original contents and signatures. Visible foreign
reasoning is converted to tagged text; the proxy never invents a signature.
Opaque Codex replay stays Codex-specific, and Claude encrypted thinking is not
sent as Codex reasoning.

Switch only at completed tool-use boundaries. Provider history validation may
still reject an unfinished or incompatible history. Context capacity can differ;
compact first if the receiving model cannot fit the current transcript.
Provider-specific in-memory continuation may reset on a switch or proxy restart.

## Other providers

After authenticating the corresponding proxy provider, explicit IDs remain
available in dual mode:

```text
/model kimi-for-coding
/model grok-4.7
/model cursor:gpt-5.5
/model opencode-go/glm-5.2
```

These are inherited capabilities, not live-tested promises from this fork.
See [provider guides](/providers/choosing-a-provider/).

## Direct Anthropic without the proxy

Start a new Claude Code process without `ANTHROPIC_BASE_URL` and without gateway
credential overrides, accounting for any settings-file values. In-session
`/model` does not change the base URL. No wrapper or global settings modification
is required for this fork's process-scoped quickstart.