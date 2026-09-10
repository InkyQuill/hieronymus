# Python archive and the Rust-only source tree

The Python 0.7.0 line is archived on Git branch `stale/python-v0.7.0`, at commit
`1cc2163`. It contains the former `src/hieronymus` application, `pyproject.toml`,
`uv.lock`, Hatch hook, Python tests and `tools` qualification/compatibility commands.
Use that branch when investigating historical source references.

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
