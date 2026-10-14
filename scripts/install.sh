#!/usr/bin/env bash
# This development fork has no published binaries. Never fetch upstream releases.
set -euo pipefail

cat <<'EOF'
claude-code-dual-subscriptions: source build required

No releases, prebuilt binaries, or Homebrew formula are published for this fork.
This helper does not download or install anything.

On Linux or Linux under WSL, install Git, a C compiler/linker and stable Rust/Cargo:
  git clone https://github.com/GranatenUdo/claude-code-dual-subscriptions.git
  cd claude-code-dual-subscriptions
  cargo build --all --locked
  export PATH="$PWD/target/debug:$PATH"

Then follow README.md for separate Claude Code and proxy Codex authentication.
Start dual mode with:
  CCP_ALIAS_PROVIDER=anthropic claude-code-proxy serve

Native Windows and macOS builds are not validated by this fork.
EOF
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  exit 0
fi
exit 1