---
title: Getting started
description: Build this fork on Linux and prepare one Claude Code session for native Claude and Codex subscriptions.
---

This is the primary development workflow for **claude-code-dual-subscriptions**.
Local mock tests passed. The user also confirmed Opus 5.5 parent + Astra subagent operation on Claude Code 2.1.299; broader live switching remains unverified.
The commands below are for a user choosing to perform live testing. Other
providers remain available in [their guides](/providers/choosing-a-provider/).

This guide uses the existing `claude-code-proxy`. The separate uncommitted
`claude-code-dual` launcher fixes workers to Astra and rejects Sol; the named Sol
agent setup below does not extend that candidate's contract.

## 1. Build this fork

Use Linux or Linux under WSL with Git, a C compiler/linker, and current stable
Rust/Cargo. On Ubuntu, the compiler prerequisites are provided by
`build-essential`. Rust 1.99 was used for this fork's checks.

```sh
git clone https://github.com/GranatenUdo/claude-code-dual-subscriptions.git
cd claude-code-dual-subscriptions
cargo build --all --locked
export PATH="$PWD/target/debug:$PATH"
claude-code-proxy --version
```

The executable retains the upstream name. The PATH change lasts for this shell.
There is no global installation. In another shell use the absolute path to the
checkout's binary, or repeat the PATH setup from this checkout.

No fork release assets or Homebrew formula are published. Do not install
upstream's binary expecting dual-subscription behavior. `scripts/install.sh`
currently prints this source-build guidance and performs no installation.
Native Windows requires the MSVC C++ build tools; native Windows and macOS
builds have not been validated in this fork.

## 2. Keep Claude Code's own subscription login

Sign in normally in Claude Code using its own login flow. Leave refresh and
credential storage to Claude Code. Do not export a dummy gateway credential,
copy tokens, or point this proxy at a keychain.

See [auth precedence](/using/configure-claude-code/) before proceeding if you
already use gateway environment variables or an `apiKeyHelper`.

## 3. Authenticate Codex separately

Use the proxy-owned login with a ChatGPT subscription, not a Platform API key:

```sh
claude-code-proxy codex auth login
# Headless alternative:
claude-code-proxy codex auth device
```

Choose one login flow. This does not change the native Codex CLI credential file.

## 4. Start the proxy

```sh
CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve
```

The listener defaults to `127.0.0.1:18765`; an interactive terminal opens the
monitor. Use `serve --no-monitor` for plain output. The built-in default remains
`codex`, so do not omit the alias-provider setting for this workflow.

## 5. Start Claude Code in another terminal

```sh
env -u ANTHROPIC_AUTH_TOKEN -u ANTHROPIC_API_KEY \
  ANTHROPIC_BASE_URL=http://127.0.0.1:18765 \
  CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
  CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK=1 \
  claude --model claude-opus-5-5
```

Settings-file environment variables and credential helpers can still override
the saved login. Review them without printing secrets. This guide does not
require changing your global Claude settings.

## 6. Keep Astra as default and optionally name a Sol agent

Follow [Astra and named Sol subagents](/using/subagents/): merge `CLAUDE_CODE_SUBAGENT_MODEL=gpt-6-astra[1m]` into project-local settings and invoke general-purpose without a per-call model argument. The user confirmed this sequence after restarting through the same proxy-enabled launcher. If a restart is needed, wait for active agents/tasks to finish.

For selected Sol tasks, leave the Astra default unchanged and use a named agent
with frontmatter `model: gpt-6.1-sol[1m]` and `effort: medium`. Confirm it is loaded
with `/agents`, then invoke its name without a per-call model override. See the
[complete example and precedence](/using/subagents/#select-sol-for-a-named-agent).
Bare `gpt-6.1` returns HTTP 400. Mixed Astra/Sol operation is user-reported, not
independently live-tested here or evidence of million-token capacity.

## 7. Optional parent-model switching

```text
/model gpt-6-astra[1m]
/model claude-opus-5-5
```

These IDs are recognized by this build; registration does not guarantee account
access. Read [switching and history](/using/switching-models-and-backends/).
The Astra example deliberately includes `[1m]` at launch/selection time for the client context setting. Read [context policy and capacity limits](/using/models-and-routing/#the-1m-hint); the successful subagent probe did not test a million-token request.
