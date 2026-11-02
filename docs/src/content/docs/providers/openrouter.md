---
title: OpenRouter
description: Use OpenRouter API-key models alongside subscription providers through the proxy.
---

OpenRouter uses its own API key and credits. The proxy sends Anthropic Messages requests to [OpenRouter's Messages endpoint](https://openrouter.ai/docs/api/api-reference/anthropic-messages/create-a-message); it does not use your Claude or Codex subscription credentials for these requests.

## Configure the proxy

Provide the key to the **proxy process** through `CCP_OPENROUTER_API_KEY` or `OPENROUTER_API_KEY`. The first nonempty value wins, followed by `openrouter.apiKey` in the proxy's `config.json`. Do not put the OpenRouter key in Claude Code's Anthropic credential variables when mixing providers.

An optional configuration block advertises favorite models:

```json
{
  "openrouter": {
    "models": ["anthropic/claude-sonnet-4"],
    "baseUrl": "https://openrouter.ai/api/v1"
  }
}
```

`models` defaults to an empty list. Use raw upstream IDs here; the proxy adds the routing prefix when listing them. Duplicates are removed. Missing credentials do not hide favorites. Invalid OpenRouter configuration disables only this provider.

`CCP_OPENROUTER_BASE_URL` overrides `baseUrl`. The default already includes `/api/v1`; the proxy appends `/messages`. URLs must use HTTPS, except loopback HTTP for local mocks, and cannot contain credentials, queries, or fragments. Redirects are not followed.

```sh
claude-code-proxy openrouter auth status
claude-code-proxy models
```

Status reports whether a key is configured and its source; it does not validate the key remotely. The `login`, `device`, and `logout` commands explain environment/file configuration and do not modify credential stores. No model catalog is fetched remotely.

## Select a model in Claude Code

Keep Claude Code pointed at your proxy and select an explicit model:

```sh
claude --model openrouter/anthropic/claude-sonnet-4
```

Use the same ID with `/model`, `ANTHROPIC_MODEL`, or `ANTHROPIC_SMALL_FAST_MODEL`. Any valid namespaced upstream ID is routable, whether or not it is a favorite. This example illustrates syntax, not current model availability.

The proxy removes exactly one `openrouter/` prefix. For OpenRouter's own `openrouter/auto` upstream ID, use `openrouter/openrouter/auto`. Suffixes such as `:free` are preserved. The existing trailing `[1m]` client hint is removed before routing; it is not a promise of upstream context capacity.

Favorites appear in CLI output and `/v1/models`. Their visibility in Claude Code's gateway picker depends on the client version; use explicit selection if they are not displayed. Existing Claude aliases, subscription routes, and session affinity do not change.

## Supported behavior and limits

- Streaming and non-streaming Messages preserve tools, thinking, images, and unknown request fields, subject to the selected upstream model's capabilities. Proxy-generated foreign thinking signatures are converted to previous-reasoning text.
- The monitor records provider/model and reported usage. OpenRouter requests have **no local cost estimate**; consult OpenRouter for billing.
- `/v1/messages/count_tokens` uses a local approximation and requires no API key or upstream call.
- Malformed, oversized, interrupted, or incomplete SSE responses are recorded as failed while the response bytes remain unchanged.
- Inbound OpenAI Chat Completions/Responses generation and the experimental `claude-code-dual` launcher are not supported by this provider.

Validation uses synthetic credentials and local fixtures. Live model compatibility and Claude Code picker behavior have not been verified. OpenRouter's [Claude Code integration guide](https://openrouter.ai/docs/cookbook/coding-agents/claude-code-integration) limits its compatibility guarantee to Anthropic's first-party provider.
