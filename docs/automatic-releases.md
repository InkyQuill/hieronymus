# Automatic releases

Conventional `fix:` commits produce patch releases; `feat:` commits produce minor releases before 1.0. Breaking changes are recorded explicitly. Release-please maintains one release PR with `version.txt`, the release manifest and CHANGELOG.md. Automation synchronizes the Rust workspace version and the three owned Cargo.lock entries before explicitly dispatching PR checks. Runtime versions still come from Cargo at build time.

Merging the release PR is the publication decision. The workflow creates the corresponding immutable `vX.Y.Z` tag, dispatches desktop-candidate once for that exact tag, waits for the matching successful run and dispatches release-rust with its run ID. Promotion consumes those verified retained bytes; installer checks use those artifacts. Missing optional native evidence remains advisory, while open P0 issues and artifact integrity checks retain their existing behavior. No installed runtime is updated.

The workflow uses GITHUB_TOKEN. GitHub suppresses ordinary workflow triggers from that token, so PR checks, candidate builds and promotion are explicitly dispatched. Actions are allowed to create release PRs; nothing auto-approves or auto-merges them. No PAT is required. Release-please itself does not create a premature empty GitHub release; the existing artifact publisher creates the release after qualification.

If a candidate fails, inspect its run and rerun the affected job rather than rebuilding unchanged successful native targets. After a retained candidate is green, recovery can dispatch release-rust manually against the same tag with `release_tag` and `candidate_run` inputs. A missing/failed candidate is never silently published.

Reference: https://github.com/googleapis/release-please-action (manifest mode and GITHUB_TOKEN workflow-trigger behavior).
