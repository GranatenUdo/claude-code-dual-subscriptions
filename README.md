# claude-code-dual-subscriptions

A development fork for switching between native Claude models using Claude Code's
saved claude.ai subscription login and GPT models using the proxy's separate
ChatGPT/Codex OAuth login, within one Claude Code session.

**Status:** 1,136 local tests passed, including eight Anthropic mock integration
tests. Live subscription use is unverified. Anthropic does not officially support
non-Claude model routing. The inherited LRU advisories remain unresolved; see
[validation](DEVELOPMENT.md) and [dependency review](DEPENDENCY-REVIEW.md).

The executable remains named `claude-code-proxy`. Its default alias provider is
still `codex` for compatibility: **explicitly enable `anthropic` for dual mode.**

## Build from source (Linux or Linux under WSL)

Install Git, a C compiler/linker (for example Ubuntu's `build-essential`), and a
current stable Rust toolchain with Cargo. The development checks used Rust 1.99.

```sh
git clone https://github.com/GranatenUdo/claude-code-dual-subscriptions.git
cd claude-code-dual-subscriptions
cargo build --all --locked
export PATH="$PWD/target/debug:$PATH"
claude-code-proxy --version
```

Keep this checkout and use its binary. This does not install the proxy globally.
There are **no fork releases, prebuilt binaries, or Homebrew formula**. Upstream
installers install upstream, which lacks this fork's changes. Native Windows and
macOS builds have not been validated here; Linux/WSL is the verified build path.

## Dual-subscription quickstart

These are manual live-use instructions, not evidence of completed live testing.
Use an account with access to the selected models. Sign in to Claude Code normally
using its own login flow first. The proxy never reads or refreshes that login.

Authenticate the proxy's separate Codex account (ChatGPT subscription, not an
OpenAI Platform API key):

```sh
claude-code-proxy codex auth login
# On a headless host: claude-code-proxy codex auth device
```

Start the proxy in that terminal:

```sh
CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve
```

In another terminal, launch Claude Code with its saved subscription login:

```sh
env -u ANTHROPIC_AUTH_TOKEN -u ANTHROPIC_API_KEY \
  ANTHROPIC_BASE_URL=http://127.0.0.1:18765 \
  CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
  CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK=1 \
  claude --model claude-opus-5
```

Do not supply a dummy auth token. An `ANTHROPIC_AUTH_TOKEN`, approved
`ANTHROPIC_API_KEY`, or `apiKeyHelper` overrides the saved Claude login.
Settings-file environment values can override the shell: inspect your existing
configuration if unsetting shell variables is insufficient. Do not log out or
copy OAuth tokens into this proxy. See [authentication setup](docs/src/content/docs/using/configure-claude-code.md).

After each turn, including tool results, finishes, switch in the same session:

```text
/model gpt-6-sol
/model claude-opus-5
```

Model access is account-dependent. Claude aliases and `claude-*` IDs stay on the
fixed Anthropic upstream even after GPT turns. GPT uses only the proxy's Codex
credentials. Native thinking signatures are preserved; foreign visible reasoning
becomes text, not fabricated signed thinking. See [switching and history](docs/src/content/docs/using/switching-models-and-backends.md).

## Documentation and other providers

The docs are maintained in this repository and can be built locally; **no fork
documentation website is hosted**. Root-relative links inside docs pages are for
the local Astro preview. Start with:

- [Getting started and source build](docs/src/content/docs/getting-started.md)
- [Models and aliases](docs/src/content/docs/using/models-and-routing.md)
- [Configuration](docs/src/content/docs/reference/configuration.md)
- [Troubleshooting](docs/src/content/docs/using/troubleshooting.md)
- [HTTP API](docs/src/content/docs/reference/http-api.md)
- [Other providers](docs/src/content/docs/providers/choosing-a-provider.md)
- [Local docs build and validation](DEVELOPMENT.md)

Kimi, Grok, OpenCode Go and Cursor functionality is retained. Their guides describe
inherited implementation behavior, not new live-account validation by this fork.
Keep the unauthenticated listener on its default `127.0.0.1` address.

## Attribution and license

Based on [raine/claude-code-proxy](https://github.com/raine/claude-code-proxy)
at `2e2975a` (v0.1.45), with native passthrough adapted from
[Sören Nikolaus's PR #133](https://github.com/raine/claude-code-proxy/pull/133),
commit `f9084b283d5f7681b023bd40f7662659eae2f337`.
Upstream history and the [MIT license](LICENSE) are retained.