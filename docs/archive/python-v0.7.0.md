# Python archive and the Rust-only source tree

The former main/Python 0.7.0 application is archived on Git branch
`stale/python-v0.7.0`, at commit `1cc2163`. That archive contains the Python
application, packaging and its tests.

Later Python compatibility and qualification tooling was introduced during the
Rust port and is absent from that branch. Recover those `tools` commands and
their tests from the immutable [pre-cleanup snapshot
`ad61ea3ef562e99b57c34e4eacd19f29b056d23c`](https://github.com/InkyQuill/hieronymus/tree/ad61ea3ef562e99b57c34e4eacd19f29b056d23c).
This snapshot remains an ancestor of the Rust-only cleanup and contains the
complete source tree immediately before removal. Use it for source references
in the retained Rust-port compatibility and qualification records.

The active application, build, test and release paths require no Python. Rust owns
the domain, CLI, daemon, MCP and native distribution; Bun 1.4.0 builds the console
and runs release acquisition/metadata helpers in `scripts`.

Historical plans, ADRs, migration proposals, reviews, compatibility snapshots and
qualification records retain their original text and source references. Commands
in those records describe the archived environment; they are not current build
instructions or release gates. Accepted Rust ADR amendments still govern behavior.
In particular, retained qualification evidence does not qualify a new native
artifact or imply implementation of a deferred agent-host matrix.

`compatibility` retains frozen data, including SQLite fixtures consumed by Rust
migration tests. `qualification/harnesses`, records, schemas and projections are
historical Rust-port evidence. The obsolete Python orchestration and its live CI
workflow were removed rather than rebuilding a deferred qualification matrix.
The pinned release assets are now acquired by `bun scripts/stage-release-assets.ts`.
Ignored `qualification/.artifacts` directories remain local and are not removed by
this cleanup. Old Python cache ignore rules remain so an existing local environment
cannot accidentally enter the repository. Local `.env` files remain ignored.
