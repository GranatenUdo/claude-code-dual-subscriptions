# claude-code-dual-subscriptions

A development fork for switching between native Claude models using Claude Code's
saved claude.ai subscription login and GPT models using the proxy's separate
ChatGPT/Codex OAuth login, within one Claude Code session.

**Status:** 1,189 local tests passed, including the Anthropic mock integration
tests, and the experimental launcher passed a local actual-client certification
matrix on Claude Code 2.1.300 with synthetic upstreams. The user confirmed Opus 5.5 parent + Astra subagent operation on Claude Code
2.1.299; broader live regression and 1M-token inference remain unverified. Anthropic does not officially support
non-Claude model routing. The inherited LRU advisories remain unresolved; see
[validation](DEVELOPMENT.md) and [dependency review](DEPENDENCY-REVIEW.md).

The existing executable remains named `claude-code-proxy`. Its default alias provider is
still `codex` for compatibility: **explicitly enable `anthropic` for dual mode.**

The local `feature/linux-dual-launcher` candidate also adds `claude-code-dual`
with `doctor`, `contract`, and `launch [--resume SESSION_ID]`. It preserves normal
project hooks, permissions and MCP; auth/model conflicts fail explicitly. It
uses an independent per-session loopback capability without dummy gateway auth.
See the [experimental launcher guide](docs/src/content/docs/using/experimental-launcher.md)
and [controlled client evidence](docs/engineering/linux-client-evidence.md).
This candidate has not been released or installed into the normal environment.
It fixes workers to Astra and rejects Sol; the named Sol recipe below applies
only to the existing `claude-code-proxy` workflow.

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

These are manual live-use instructions; the confirmed scope is the subagent setup below.
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
  claude --model claude-opus-5-5
```

Do not supply a dummy auth token. An `ANTHROPIC_AUTH_TOKEN`, approved
`ANTHROPIC_API_KEY`, or `apiKeyHelper` overrides the saved Claude login.
Settings-file environment values can override the shell: inspect your existing
configuration if unsetting shell variables is insufficient. Do not log out or
copy OAuth tokens into this proxy. See [authentication setup](docs/src/content/docs/using/configure-claude-code.md).

After each turn, including tool results, finishes, switch in the same session:

```text
/model gpt-6-astra[1m]
/model claude-opus-5-5
```

Model access is account-dependent. Claude aliases and `claude-*` IDs stay on the
fixed Anthropic upstream even after GPT turns. GPT uses only the proxy's Codex
credentials. Native thinking signatures are preserved; foreign visible reasoning
becomes text, not fabricated signed thinking. See [switching and history](docs/src/content/docs/using/switching-models-and-backends.md).

## Working Opus parent + Astra subagent setup

Merge this into the existing project-local `.claude/settings.local.json` environment without replacing unrelated settings:

~~~json
{
  "env": {
    "CLAUDE_CODE_SUBAGENT_MODEL": "gpt-6-astra[1m]"
  }
}
~~~

If the changed default requires a restart, wait for active agents/tasks to finish, then restart Claude Code through the **same proxy-enabled launcher**. Use the built-in general-purpose subagent with **no per-call model argument**. On the user's 2.1.299 installation that argument accepts only Claude aliases, so a full GPT ID there is rejected. No custom Astra agent type is needed. Leave `CLAUDE_CODE_SUBAGENT_MODEL_FORCE` unset so an explicit `opus` reviewer can retain its selection; that live reviewer probe is not yet verified.

The user confirmed the post-restart transcript recorded `gpt-6-astra`; the pre-restart probe used Opus. `[1m]` requests Claude Code's 1M context assumption for this custom ID, and the proxy strips it before upstream routing. This does not establish the account's actual capacity or prove million-token inference. Check old compaction overrides before relying on the larger window.

See [subagent setup, verification and undo](docs/src/content/docs/using/subagents.md) and [precise context behavior](docs/src/content/docs/using/models-and-routing.md#the-1m-hint).

For a named Sol agent, keep that Astra default unchanged and set the agent's
frontmatter to `model: gpt-6.1-sol[1m]` and `effort: medium`. Use `/agents` to
confirm the definition is loaded, then invoke its name without a per-call model
override. Agent loading is separate from routing; do not interrupt active work
for a restart. The exact `gpt-6.1-sol` ID is required: `gpt-6.1` returns HTTP 400.
On 2.1.299, selection order is per-call, definition, environment, then parent;
effort overrides can also affect the final request. See the
[complete named-agent example](docs/src/content/docs/using/subagents.md#select-sol-for-a-named-agent).
The user reported mixed Astra/Sol success without changing the environment
default. It has not been independently live-tested here and is not proof of 1M
capacity. Verify provider/model records and transcripts, not model self-reports.

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

Kimi, OpenCode Go and Cursor functionality is retained. Their guides describe
inherited implementation behavior, not new live-account validation by this fork.
Keep the unauthenticated listener on its default `127.0.0.1` address.

## Attribution and license

Based on [raine/claude-code-proxy](https://github.com/raine/claude-code-proxy)
at `2e2975a` (v0.1.45), with native passthrough adapted from
[Sören Nikolaus's PR #133](https://github.com/raine/claude-code-proxy/pull/133),
commit `f9084b283d5f7681b023bd40f7662659eae2f337`.
Upstream history and the [MIT license](LICENSE) are retained.
