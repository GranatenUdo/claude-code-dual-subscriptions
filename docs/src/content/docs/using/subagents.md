---
title: Opus parent, Astra default and named Sol agents
description: Keep Astra as the default and select Sol for a named agent in the existing proxy workflow.
---

The user confirmed this setup works on **Claude Code 2.1.299**: an Opus 5.5 parent and a built-in general-purpose subagent using `gpt-6-astra[1m]`. Before restarting, the probe still used Opus; after restarting through the same proxy-enabled launcher, its transcript recorded `gpt-6-astra`. This is evidence of that real operation, not a complete live regression, security clearance, or 1M-token inference test.

## Merge the subagent default

This guide uses the existing `claude-code-proxy` workflow. The separate, uncommitted [experimental `claude-code-dual` launcher](/using/experimental-launcher/) has a fixed Astra worker contract and rejects Sol requests. Its contract has not been extended by this documentation update.

In the target project's `.claude/settings.local.json`, merge this key into the existing `env` object. Do not replace the file or discard permissions, hooks, other environment keys, or existing settings. This is an example for a user to apply; repository development does not edit their project settings.

~~~json
{
  "env": {
    "CLAUDE_CODE_SUBAGENT_MODEL": "gpt-6-astra[1m]"
  }
}
~~~

For a process-only alternative, put the same value in the existing proxy-enabled launcher:

~~~sh
env -u ANTHROPIC_AUTH_TOKEN -u ANTHROPIC_API_KEY \
  ANTHROPIC_BASE_URL=http://127.0.0.1:18765 \
  CLAUDE_CODE_SUBAGENT_MODEL='gpt-6-astra[1m]' \
  CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
  CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK=1 \
  claude --model claude-opus-5-5
~~~

Keep the server running with `CCP_ALIAS_PROVIDER=anthropic`. Keep Claude Code's native login and the proxy's separate Codex login. See [authentication precedence](/using/configure-claude-code/). Use the same launcher, port and environment that already work for your account. The observed session needed a restart to pick up a changed default. If a restart is needed, wait until active agents and tasks have finished, then restart from the same project. Adding a named Sol agent does not require changing this Astra default.

## Invoke the right agent

Ask the parent to use **general-purpose**, with **no per-call model argument**, for the Astra task. On the observed 2.1.299 installation, the Agent tool's model parameter rejected full IDs and accepted only `sonnet|opus|haiku|fable`. Passing `gpt-6-astra[1m]` there fails before the proxy sees a request. The environment default is the working workaround. An agent-type name is not a model ID; no custom `astra` agent type is needed.

For this Claude Code 2.1.299 setup, the [subagent model order](https://code.claude.com/docs/en/sub-agents#choose-a-model) is per-call model, then agent-definition model, then environment default, then parent model. Thus a reviewer requested with per-call `model: "opus"` can select Claude instead. That reviewer path has **not** been verified by the reported live probe. Leave `CLAUDE_CODE_SUBAGENT_MODEL_FORCE` unset: it defeats these exceptions. A definition using `inherit` can also prevent the Astra default from being selected.

The proxy's literal `opus` alias maps to `claude-opus-5-5`; Claude Code can resolve aliases before sending requests. The parent example uses the explicit `claude-opus-5-5` ID. Check the actual reviewer request rather than assuming an alias guarantees 5.5.

## Select Sol for a named agent

The user subsequently reported successfully spawning Sol agents without changing the Astra environment default. This mixed Astra/Sol result is user-reported; repository development has not independently live-tested it or demonstrated 1M-token capacity.

For the existing proxy workflow, a named agent definition can select Sol while general-purpose continues using Astra. Example project file `.claude/agents/gpt61-impl.md` (an example only; this repository does not create or edit your personal/project agent files):

~~~markdown
---
name: gpt61-impl
description: Use for implementation tasks explicitly assigned to this agent.
model: gpt-6.1-sol[1m]
effort: medium
---
Carry out the assigned implementation task and report changes and verification.
~~~

Use the exact ID `gpt-6.1-sol[1m]`. The proxy strips `[1m]` and routes `gpt-6.1-sol` to Codex. Bare `gpt-6.1` (including `gpt-6.1[1m]`) is not registered and returns HTTP 400; it is not an alias for Sol. No global model-default change or `CCP_CODEX_MODEL` override is needed. A configured Codex model override can replace an otherwise valid model selection, so account for it when verifying routing.

Use `/agents` to verify that Claude Code has loaded the named definition. Agent discovery/reload is separate from provider routing: a definition that is not loaded cannot be selected even though its model is supported by the proxy. Use the client's available reload mechanism; if a restart is required, wait for a safe time after active work finishes. Do not restart active agents to apply this example.

Ask the parent to use **gpt61-impl**, with **no per-call model argument**. Keep `CLAUDE_CODE_SUBAGENT_MODEL=gpt-6-astra[1m]` unchanged and FORCE unset. The definition's explicit model takes precedence over the Astra default. `effort: medium` requests medium effort; higher-priority client effort settings and the proxy's `CCP_CODEX_EFFORT`/Codex effort configuration can override the effective value. Verify the request rather than assuming the frontmatter guarantees the final effort.

## Verify routing evidence

When choosing to perform live verification, ask for a brief, harmless general-purpose response and a separate named Sol agent response, omitting per-call model arguments. These requests use the account. Correlate their request times with the proxy monitor and inspect the recorded provider/model and subagent transcripts. Expect Codex `gpt-6-astra` for general-purpose and Codex `gpt-6.1-sol` for the named agent; the parent should remain native Claude. A model's self-description is not verification. The suffix is stripped by this proxy, so its absence in the monitor is expected. This documentation update did not run these live probes.

Use Claude Code's `/tasks` model display as another check. A tiny successful probe establishes routing and response delivery only. It does not demonstrate acceptance of a million-token prompt. See [the context policy](/using/models-and-routing/#the-1m-hint).

## Troubleshoot and undo

- Still Opus or Astra for the named Sol agent: check `/agents`, per-call arguments, definition-level model/inherit, FORCE and higher-priority settings. If a restart is needed, wait for active work to finish.
- Full-ID enum error: remove the Agent tool model argument; keep the full ID in the environment default for Astra or the named agent's frontmatter for Sol.
- Unknown `gpt-6.1`: use the exact `gpt-6.1-sol` ID. Sol rejected by `claude-code-dual`: that candidate still restricts workers to Astra; this recipe is for the existing proxy.
- Model rejected/substituted: inspect `availableModels`, managed restrictions, alias pins and fallback warnings. Do not bypass organizational policy; the intended ID must be allowed.
- Missing proxy request: check the restarted process retained `ANTHROPIC_BASE_URL` and the intended proxy port.
- Reviewer unexpectedly Astra: check for `CLAUDE_CODE_SUBAGENT_MODEL_FORCE` and remove the override you added before restarting.

To undo the Sol example, restore or remove only the named agent definition you added; leave the Astra default in place. To undo the Astra default too, restore only its previous key/value (or remove your addition). Reload or restart at a safe time through the existing launcher. Preserve unrelated settings. Removing a project key does not cancel a shell or higher-priority default. No global settings or credential migration is required.
