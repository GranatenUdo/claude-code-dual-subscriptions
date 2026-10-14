---
title: Choosing a provider
description: Compare claude-code-proxy providers by account, protocol, models, reasoning, multimodal input, tools, and operational tradeoffs.
---

Start with native Claude plus Codex for this fork: run the server with `CCP_ALIAS_PROVIDER=anthropic`. Keep Claude Code logged in to claude.ai and authenticate Codex separately. Other providers remain available on the same listener; their details below describe inherited functionality, not live-account validation by this fork.

| Provider | Account | Upstream protocol | Model selection | Notable capabilities |
| --- | --- | --- | --- | --- |
| [Anthropic](/providers/anthropic/) (opt-in) | Claude Code subscription login or Anthropic API credentials | Anthropic Messages passthrough | `claude-*` and built-in Claude aliases | Native tools, thinking and streaming; credentials forwarded from Claude Code |
| [Codex](/providers/codex/) | ChatGPT Plus or Pro | OpenAI Responses over WebSocket or HTTP SSE | Named Codex catalog, `-fast` and `-ultrafast` variants | Function tools, image input, hosted web search, reasoning summaries, optional native Responses route |
| [Kimi](/providers/kimi/) | kimi.com with Kimi Code access | OpenAI-style chat completions | `kimi-for-coding` and aliases | Function tools, reasoning, image and video input |
| [OpenCode Go](/providers/opencode-go/) | OpenCode Go subscription | OpenAI-compatible Chat Completions, OpenAI Responses, or Anthropic-compatible Messages | Non-conflicting bare IDs and `opencode-go/<model-id>` forms | Inherited curated model catalog |
| [Cursor Agent](/providers/cursor-agent/) | Cursor account | HTTP/2 Connect stream | Cursor modes and `cursor:<model-id>` prefixes | Dynamic model catalog, effort variants, images, plan and ask modes, session continuation |

## Practical guidance

- Start with **Anthropic and Codex together** for the dual-subscription workflow. See [Getting started](/getting-started/).
- Choose **Kimi** for the Kimi Code model and multimodal coding input.
- Choose **OpenCode Go** when you have a Go subscription and want its documented model catalog in Claude Code.
- Choose **Cursor Agent** when you want Cursor's model catalog and agent modes. It depends on an installed Cursor Agent bundle for protobuf schemas.

## Shared behavior

All providers route by model ID, stream responses and report failures through the same Anthropic-shaped API. Translating providers use proxy-owned credentials and refresh tokens when supported. Anthropic forwards Claude Code's credentials and native request fields instead.

<div class="security-callout">
<strong>Account policy matters.</strong> Provider subscriptions, model access, regional availability, rate limits, and rules for unofficial clients can change. Review the terms for your account before using a provider through the proxy.
</div>

Use `CCP_ALIAS_PROVIDER=anthropic claude-code-proxy models` for the current compact catalog and `claude-code-proxy models --full` for all advertised aliases. The [Models and routing](/using/models-and-routing/) page explains aliases and discovery.
