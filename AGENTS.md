# Agent working rules

These rules apply to every coding agent working in this repository (Claude Code,
Codex and others). Claude Code loads this file only while no `CLAUDE.md` exists;
a future `CLAUDE.md` must import it with `@AGENTS.md`.

## Every plan is adversarially reviewed by an agent

Before any plan is presented for approval or executed, a separate **read-only**
agent with no authorship stake (a subagent or a fresh session) must review it
adversarially. The reviewer tries to break the plan against the actual code and
repository state: false assumptions, missing steps, unsafe, irreversible or
outward-facing actions, scope creep, unverifiable claims and test gaps.

- The author resolves every finding or records it as an explicit open risk.
- The plan contains an **Adversarial review** section: who reviewed, the
  findings, and how each was resolved.
- A material revision of the plan is reviewed again before approval.

## Scope boundaries

Without separate, explicit user authorization, do not:

- make live subscription or provider calls;
- read real secrets, tokens or credential stores;
- edit normal Claude Code or Codex settings, or install into the normal
  environment;
- access my-office;
- publish releases.

Actual-client tests use synthetic homes and credentials inside bwrap network and
filesystem namespaces. Preserve project safety hooks, permissions and MCP.

Each `git push`, PR creation, marking a PR ready and merge needs explicit user
confirmation; one confirmation may cover several of these only if it names each
one.

Never bind the default proxy port `18765`, reuse the real proxy configuration in
tests, or build into a target directory that a running proxy executes from. Use
a separate `CARGO_TARGET_DIR` and temporary `HOME`/`XDG_*`/`CCP_CONFIG_DIR`.

## Required checks

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --locked -- -D warnings`
- `cargo build --all --locked`
- `cargo test --all --locked`
- `python3 -m unittest tests/test_dual_installer.py tests/test_dual_tty.py`
  (build `claude-code-dual` first; `CCD_BINARY` overrides its path)
- Docs: `cd docs && bun install --frozen-lockfile && bun run build && bun run check:links`

Report each check as passed, failed or not run, and record `rustc -V`.

## Pricing table

API-equivalent cost estimates in the monitor use the hardcoded table in
`src/pricing.rs`, together with `PRICES_AS_OF` and the source URLs. When prices
change, update the table, the date and the pricing tests together.
