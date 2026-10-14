---
title: Getting started
description: Build this fork on Linux and prepare one Claude Code session for native Claude and Codex subscriptions.
---

This is the primary development workflow for **claude-code-dual-subscriptions**.
It has been tested with local mock servers; live account switching is unverified.
The commands below are for a user choosing to perform live testing. Other
providers remain available in [their guides](/providers/choosing-a-provider/).

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
  claude --model claude-opus-5
```

Settings-file environment variables and credential helpers can still override
the saved login. Review them without printing secrets. This guide does not
require changing your global Claude settings.

## 6. Switch after a completed turn

```text
/model gpt-6-sol
/model claude-opus-5
```

These IDs are recognized by this build; registration does not guarantee account
access. Read [switching and history](/using/switching-models-and-backends/).
Do not apply a one-million-token context hint indiscriminately across providers.
No fixed cross-provider context limit is asserted by this quickstart.