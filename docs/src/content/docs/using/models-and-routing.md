---
title: Models and routing
description: Understand provider model patterns, aliases, Codex fast mode, context hints, small models, model listing, and Claude Code gateway discovery.
---

The model ID in each request selects its provider. For dual subscriptions explicitly set `CCP_ALIAS_PROVIDER=anthropic`. Native Claude is available only through the Anthropic Messages and token-count routes, not the OpenAI-compatible routes.

## Routing patterns

| Pattern | Provider |
| --- | --- |
| Registered `gpt-*` IDs and their `-fast` and `-ultrafast` forms | Codex |
| `kimi-for-coding`, `kimi-k2.6`, `k2.6`, `kimi-k3`, `k3` | Kimi |
| `grok-composer-2.5-fast`, `grok-4.5`, `grok-4.6`, `grok-4.7` | Grok |
| Non-conflicting registered OpenCode Go IDs and every `opencode-go/<model-id>` | OpenCode Go |
| `cursor`, Cursor legacy aliases, `cursor:<id>`, `cursor-plan:<id>`, `cursor-ask:<id>` | Cursor Agent |
| Anthropic-style aliases such as `haiku`, `sonnet`, `opus`, `fable`, and registered `claude-*` aliases | Anthropic in dual mode; legacy built-in default is Codex |

In dual mode, any `claude-*` ID is forwarded to Anthropic for validation. Other unknown IDs return HTTP 400 with the supported provider catalog. There is no implicit fallback for arbitrary model names.

Native short aliases resolve as follows: `haiku` → `claude-haiku-4-5`, `sonnet` → `claude-sonnet-5`, `opus` → `claude-opus-5`, `fable` → `claude-fable-5`. Native Claude also bypasses auto-review overrides.

## Prefer the local catalog

The model list changes faster than documentation. Ask the installed CLI:

```sh
CCP_ALIAS_PROVIDER=anthropic claude-code-proxy models
CCP_ALIAS_PROVIDER=anthropic claude-code-proxy models --full
```

The compact command abbreviates Cursor's dynamic aliases. `--full` prints every alias discovered through the installed Cursor Agent catalog.

The HTTP equivalent is:

```sh
curl http://127.0.0.1:18765/v1/models
```

## Codex fast mode

Every registered Codex model also has a local `-fast` form. The proxy removes `-fast` from the upstream model and requests the priority service tier.

Models that offer the ultrafast tier also have a local `-ultrafast` form, which requests the ultrafast service tier. Today those are `gpt-6-astra-ultrafast` and `gpt-6.1-sol-ultrafast`. An `-ultrafast` name on any other model is not a registered ID.

For Claude Code requests on `/v1/messages`, a configured `codex.serviceTier` or `CCP_CODEX_SERVICE_TIER` override wins over both suffixes. When the forced tier is `ultrafast` and the model does not offer it, the proxy requests priority instead.

## The `[1m]` hint

A trailing `[1m]` affects Claude Code's local compaction policy. The proxy strips it before matching and forwarding the model. Use it only when the upstream model and account can accept the resulting context, and set a safe `CLAUDE_CODE_AUTO_COMPACT_WINDOW` when the real limit is below one million tokens.

## Main and small models

Set both model variables to routable IDs:

```sh
ANTHROPIC_MODEL=claude-opus-5
ANTHROPIC_SMALL_FAST_MODEL=claude-haiku-4-5
```

Claude Code sends background work to its small/fast model. Its built-in Haiku ID can route through `aliasProvider`, but a concrete provider ID keeps the behavior explicit.

## Claude Code `/model`

When `ANTHROPIC_BASE_URL` already points to the proxy, these can change the request model:

- `claude --model <id>` at launch
- Claude Code's `/model` command in a session
- `ANTHROPIC_MODEL` for a new process

A model change can switch upstream providers because routing is per request. It does not change `ANTHROPIC_BASE_URL` or move the process back to direct Anthropic.

## Gateway model discovery

Enable Claude Code discovery at process start:

```sh
env -u ANTHROPIC_AUTH_TOKEN -u ANTHROPIC_API_KEY \
CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY=1 \
ANTHROPIC_BASE_URL=http://127.0.0.1:18765 \
ANTHROPIC_MODEL=claude-opus-5 \
ANTHROPIC_SMALL_FAST_MODEL=claude-haiku-4-5 \
  claude
```

`/v1/models` includes raw provider IDs and Anthropic-style aliases. Picker behavior depends on the Claude Code version; discovery does not prove live account access. Raw IDs remain usable through `--model`, `/model`, and environment variables.

## Alias routing

`CCP_ALIAS_PROVIDER=kimi` or `"aliasProvider": "kimi"` routes recognized Anthropic-style aliases to Kimi. Accepted values are `codex`, `kimi`, and `anthropic`. Registered Claude IDs are also legacy aliases when native mode is not enabled. Explicit GPT and other provider IDs retain their provider.

With `anthropic`, Claude aliases and explicit `claude-*` IDs go to [Anthropic](/providers/anthropic/) using the credentials Claude Code sends. They remain on Anthropic even after a GPT or Kimi turn establishes session affinity. Existing `codex` and `kimi` alias behavior is unchanged.
