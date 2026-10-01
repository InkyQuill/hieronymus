# Repository readiness audit — 2026-10-01

Starting main: `2ba73bfc1b10e509155bdf9c6c0c90aded2dbd12`, identical to fetched origin/main. All six secondary existing worktrees were clean. Nine missing temporary worktree registrations were stale.

## Branch reconciliation

- `agent/remediation-program`: abandoned Python remediation line from August; superseded by the Rust-only cutover (`36ea66d`, PR #23). Preserve history without restoring Python.
- `feat/rust-rewrite`: alternative early Rust port; current main uses the subsequently reviewed `feat/rust-rewrite-proposals` implementation (`fcab0b5` and subsequent correctness merges). Restoring this branch would replace current schema, domain, daemon and release contracts with the older design. Preserve its commits without replacing the active implementation.
- `codex/macos-stop-release`: squash-merged as PR #34 (`9153646`); retain current tree.
- `codex/installer-diagnostics`: temporary Windows diagnostic workflow pinned to run 34755165337 and staging version 0.9.1. It intentionally throws after its probe and is unsuitable as active CI. Preserve the unique final script under historical-diagnostics, without restoring the obsolete workflow.
- `codex/v0.9.0-qualification`, `codex/v0.9.1-qualification`, `codex/release-v0.9.2-qualification`: unique historical candidate evidence; retain each snapshot separately under historical-evidence. These records do not qualify current main or a new release.
- Other existing worktree heads are already ancestors of main.

The replaced branches are joined with an explicit history-only merge after preserving unique historical materials. All original commits remain reachable from main. This is not a claim that their alternative implementations are shipped.

## Issues

Read all 19 issue bodies and all issue comments: 15 open, 4 closed. No issues were changed or closed during this preparation. This is backlog triage, not a fresh reproduction of each defect.

| Issue | State | Title |
| --- | --- | --- |
| [#55](https://github.com/InkyQuill/hieronymus/issues/55) | open | Bound prompt hook retains unrelated technical messages before relevance filtering |
| [#54](https://github.com/InkyQuill/hieronymus/issues/54) | open | Trusted hook hides structured authority conflicts as unexpected response |
| [#53](https://github.com/InkyQuill/hieronymus/issues/53) | open | CWS: no layout-aware journaled relocation for existing canonical projects |
| [#52](https://github.com/InkyQuill/hieronymus/issues/52) | open | Linux uninstall не очищает CLI/desktop/service и agent hook-регистрации; устаревший Codex config остаётся |
| [#51](https://github.com/InkyQuill/hieronymus/issues/51) | open | Codex: глобальный prompt-хук требует binding в непрофильных проектах без MCP-контекста |
| [#47](https://github.com/InkyQuill/hieronymus/issues/47) | open | Убрать механизм proposals и обязательные ручные ревью из агентского флоу |
| [#46](https://github.com/InkyQuill/hieronymus/issues/46) | open | Документация: нет схемы stdin-контекста version:1 для `hiero agent-hook bind-context` |
| [#45](https://github.com/InkyQuill/hieronymus/issues/45) | open | hiero update has no default release source: every invocation needs HIERONYMUS_RELEASE_URL or --release-url/--release-dir |
| [#44](https://github.com/InkyQuill/hieronymus/issues/44) | open | hiero update never returns when stdout is a pipe: spawned daemon inherits the invocation's stdio |
| [#43](https://github.com/InkyQuill/hieronymus/issues/43) | open | hiero update --release-url cannot consume a GitHub Releases URL: channel path segment appended, redirects refused, failure reported as "model download failed" |
| [#42](https://github.com/InkyQuill/hieronymus/issues/42) | open | Release warning: release-rust / promote / all |
| [#41](https://github.com/InkyQuill/hieronymus/issues/41) | open | P2: CI client-discovery startup test times out waiting for mock manager |
| [#40](https://github.com/InkyQuill/hieronymus/issues/40) | open | P2: argv routing tests discover the user daemon instead of isolating data roots |
| [#38](https://github.com/InkyQuill/hieronymus/issues/38) | closed | P2: Full local Rust test linking oversubscribes memory and stalls |
| [#37](https://github.com/InkyQuill/hieronymus/issues/37) | open | P2: Verify native config defaults and advisory installer release checks |
| [#36](https://github.com/InkyQuill/hieronymus/issues/36) | open | P2: Successful installer hides the legacy database migration warning |
| [#35](https://github.com/InkyQuill/hieronymus/issues/35) | closed | what is this update check? |
| [#33](https://github.com/InkyQuill/hieronymus/issues/33) | closed | Dream batches exceed Ollama context: silent prompt truncation causes repeated coverage_audit failures |
| [#31](https://github.com/InkyQuill/hieronymus/issues/31) | closed | Installed stdio MCP adapter rejects initialize with mirrored metadata Header mismatch |

Suggested order: hook relevance/lifecycle and diagnostics (#51, #55, #54, #46); uninstall ownership (#52); update transport/defaults/stdio (#43–45); autonomous memory flow (#47); test isolation (#40–41); installer warnings and native edge cases (#36–37); historical evidence gap (#42). #53 concerns CWS relocation and requires routing to its canonical implementation rather than guessing a Hieronymus core fix.

## Local build storage

Initial target: 7.1 GiB; debug: 6.4 GiB; incremental: 5.9 GiB. No cargo/rustc process was running. Remove only disposable debug artifacts, retain release binaries and published artifacts. Workspace dev/test profiles disable debug symbols and incremental compilation; .cargo/config.toml limits build jobs to two. This matches existing CI resource settings. Cargo target files have no automatic size cap: monitor `du -sh target/debug`; clean the dev profile when obsolete outputs accumulate. These settings mitigate the dominant growth sources, not a guarantee of a fixed maximum.
