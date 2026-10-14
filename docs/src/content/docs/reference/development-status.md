---
title: Development status
description: Validation evidence, distribution limits and outstanding dependency advisories.
---

This is [GranatenUdo/claude-code-dual-subscriptions](https://github.com/GranatenUdo/claude-code-dual-subscriptions), a development fork of raine/claude-code-proxy. The original MIT license, history and upstream attribution are retained. The baseline is `2e2975aee754fff176765bcc2f3e582a79191751` (upstream v0.1.45); native passthrough adapts upstream PR #133. The implementation commit is `ecfc35cf19d96323030c11be8acf53ddf2ac45eb`.

## Verified locally

On 2026-10-14, Linux/WSL passed 1,136 tests with zero failures, including eight native Anthropic mock-server integration tests. Coverage includes route switching, auth isolation, native errors, redirects, streaming/cancellation, count_tokens and mixed signed history. Existing provider regression tests passed. Formatting, strict Clippy and the locked build passed:

~~~sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo build --all --locked
cargo test --all --locked
~~~

## Not verified or distributed

The implementation/test agent made no live subscription requests. Account access, token refresh and live cross-provider history acceptance need separately approved testing. Non-Claude routing is not officially supported by Anthropic. Native Windows compilation was blocked by a missing MSVC linker; Windows and macOS builds are not certified here.

No fork releases, prebuilt binaries, package formula or hosted documentation are published. Build this fork from source using the [quickstart](/getting-started/). Upstream release assets do not contain this implementation. Documentation CI only builds and checks links; it does not deploy a website.

## Dependency limitation

The inherited `lru 0.12.5` dependency remains affected by RUSTSEC-2026-0002 (fixed from 0.16.3) and RUSTSEC-2026-0253 (fixed from 0.18.2). Ratatui 0.29 requires the older series, so this requires a separately tested migration or backport. See [DEPENDENCY-REVIEW.md](https://github.com/GranatenUdo/claude-code-dual-subscriptions/blob/main/DEPENDENCY-REVIEW.md). Passing mock tests does not resolve these advisories.

## Build the documentation locally

From the repository root, with Bun 1.3.13 or compatible installed:

~~~sh
cd docs
bun install --frozen-lockfile
bun run build
bun run check:links
bun run dev
~~~

The preview uses localhost:4321; generated links refer to that local preview. No external hosting setup is needed.

## User-confirmed operation: Claude Code 2.1.299

The user reports a working Opus 5.5 parent and general-purpose Astra subagent after setting `CLAUDE_CODE_SUBAGENT_MODEL=gpt-6-astra[1m]` in project-local settings and restarting through the same proxy-enabled launcher. The pre-restart probe used Opus; the post-restart transcript recorded `gpt-6-astra`. No full-ID per-call model argument was used.

This evidence does not verify the explicit Opus reviewer exception, million-token inference, all refresh/history paths, or a complete live regression/security review. The unresolved dependency advisories remain. This documentation update makes no provider calls or user-project settings changes.
