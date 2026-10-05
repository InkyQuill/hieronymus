# Automatic releases

Conventional `fix:` commits produce patch releases; `feat:` commits produce minor releases before 1.0. Breaking changes are recorded explicitly. Release-please maintains one release PR with `version.txt`, the release manifest and CHANGELOG.md. Automation synchronizes the Rust workspace version and the three owned Cargo.lock entries before explicitly dispatching PR checks. Runtime versions still come from Cargo at build time.

Merging the release PR is the publication decision. The workflow creates the corresponding immutable `vX.Y.Z` tag, dispatches desktop-candidate once for that exact tag, waits for the matching completed run and dispatches release-rust with its run ID. Promotion consumes those verified retained bytes; installer checks use those artifacts. Missing optional native evidence remains advisory, while open P0 issues and artifact integrity checks retain their existing behavior. No installed runtime is updated.

The workflow uses GITHUB_TOKEN. GitHub suppresses ordinary workflow triggers from that token, so PR checks, candidate builds and promotion are explicitly dispatched. Actions are allowed to create release PRs; nothing auto-approves or auto-merges them. No PAT is required. Release-please itself does not create a premature empty GitHub release; the existing artifact publisher creates the release after qualification.

If a candidate fails, inspect its run and rerun the affected job rather than rebuilding unchanged successful native targets. Recovery can dispatch release-rust manually against the same tag with `release_tag` and `candidate_run` inputs once retained artifacts are available. Non-P0 check failures remain disclosed warnings; promotion still requires valid available artifacts. A missing artifact or integrity failure is never silently treated as success.

Reference: https://github.com/googleapis/release-please-action (manifest mode and GITHUB_TOKEN workflow-trigger behavior).

Non-breaking `refactor:` commits are release-worthy patch changes, like `fix:`.
They appear under “Refactoring and Build Simplification”; `feat:` still requests
minor, and explicit breaking changes retain the configured pre-1.0 behavior.
Docs/chore-only work does not request a release. Configuration changes prepare
future release PRs; publication still requires merging the version PR.

Release notes describe user-visible changes and actual qualification gaps for
that version. Completed milestone announcements belong in CHANGELOG.md, not
in standing instructions for an unspecified next release.
