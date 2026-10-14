---
title: Opus parent and Astra subagents
description: User-confirmed Claude Code 2.1.299 setup with native Opus 5.5 and a Codex Astra subagent.
---

The user confirmed this setup works on **Claude Code 2.1.299**: an Opus 5.5 parent and a built-in general-purpose subagent using `gpt-6-astra[1m]`. Before restarting, the probe still used Opus; after restarting through the same proxy-enabled launcher, its transcript recorded `gpt-6-astra`. This is evidence of that real operation, not a complete live regression, security clearance, or 1M-token inference test.

## Merge the subagent default

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

Keep the server running with `CCP_ALIAS_PROVIDER=anthropic`. Keep Claude Code's native login and the proxy's separate Codex login. See [authentication precedence](/using/configure-claude-code/). Use the same launcher, port and environment that already work for your account. Fully exit and restart Claude Code from the same project after changing the default; an already-running session did not pick it up in the observed setup.

## Invoke the right agent

Ask the parent to use **general-purpose**, with **no per-call model argument**, for the Astra task. On the observed 2.1.299 installation, the Agent tool's model parameter rejected full IDs and accepted only `sonnet|opus|haiku|fable`. Passing `gpt-6-astra[1m]` there fails before the proxy sees a request. The environment default is the working workaround. An agent-type name is not a model ID; no custom `astra` agent type is needed.

The [official subagent model order](https://code.claude.com/docs/en/sub-agents#choose-a-model) places explicit invocation and definition models ahead of the environment default. Thus a reviewer requested with per-call `model: "opus"` can select Claude instead. That reviewer path has **not** been verified by the reported probe. Do not set `CLAUDE_CODE_SUBAGENT_MODEL_FORCE`: it defeats that exception. A definition using `inherit` can also prevent the Astra default from being selected.

The proxy's literal `opus` alias currently maps to `claude-opus-5`; Claude Code can resolve aliases before sending requests. The parent example uses the explicit `claude-opus-5-5` ID. Check the actual reviewer request rather than assuming an alias guarantees 5.5.

## Verify a probe yourself

After restart, ask for one brief, harmless general-purpose response, explicitly omitting the model argument. This makes a live request and uses the account. Correlate its request time with the proxy monitor and inspect the recorded provider/model and subagent transcript. The Astra request should route through Codex as `gpt-6-astra`; the parent should remain native Claude. A model's self-description is not verification. The suffix is stripped by this proxy, so its absence in the monitor is expected.

Use Claude Code's `/tasks` model display as another check. A tiny successful probe establishes routing and response delivery only. It does not demonstrate acceptance of a million-token prompt. See [the context policy](/using/models-and-routing/#the-1m-hint).

## Troubleshoot and undo

- Still Opus: restart through the same launcher; check per-call model arguments, definition-level model/inherit, and higher-priority settings.
- Full-ID enum error: remove the Agent tool model argument; keep the full ID in `CLAUDE_CODE_SUBAGENT_MODEL`.
- Model rejected/substituted: inspect `availableModels`, managed restrictions, alias pins and fallback warnings. Do not bypass organizational policy; the intended ID must be allowed.
- Missing proxy request: check the restarted process retained `ANTHROPIC_BASE_URL` and the intended proxy port.
- Reviewer unexpectedly Astra: check for `CLAUDE_CODE_SUBAGENT_MODEL_FORCE` and remove the override you added before restarting.

To undo, restore only the previous subagent-default key/value (or remove your addition) and restart through the existing launcher. Preserve unrelated settings. Removing a project key does not cancel a shell or higher-priority default. No global settings or credential migration is required.
