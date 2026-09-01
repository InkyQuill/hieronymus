# Rust qualification

This directory contains isolated, test-only evidence for the proposed Rust migration. It is
not a production Rust workspace. Qualification failures block the dependent implementation
plan; they are not fixed by weakening the frozen compatibility contracts.

Prerequisites are pinned in `prerequisites.json` and `rust-toolchain.toml`. Network access is
permitted only for the explicit acquisition commands listed in the prerequisites file. Normal
tests use fake executables and never acquire Rust, Bun, models, or native libraries. The single
Cargo environment smoke is opt-in with `HIERONYMUS_QUALIFICATION_LIVE=1` and uses the already
installed Rust 1.96.0 toolchain in offline, no-auto-install mode.

All transient work, logs, installation roots, Cargo targets, and frontend bundles live under
the ignored `qualification/.artifacts/` root. Live work roots must already exist as private,
current-user directories strictly below that canonical root; tests may instead use an equivalent
private root below the canonical system temporary directory. The environment builder never creates
or changes the caller's work root and creates only descriptor-relative, no-follow descendants.

`python -m tools.qualification.clean` is a dry run. Pass `--apply` to remove only the bounded
transient allowlist. Acquired models are retained unless `--include-model` is also explicit. Dry
run and apply perform the same complete descriptor-relative validation, including filesystem device
and Linux mount identities. Cleanup refuses symlinks, bind mounts, hard links, special files,
identity swaps, repository/home roots, and the translation workspace.

Every process run gets a fresh, single-purpose Python supervisor. The caller passes the already
sanitized argv, working directory, environment, and time bounds only through inherited anonymous
pipes; none of those values appear in command lines, files, logs, errors, or receipts. The
supervisor is single-threaded, enables Linux subreaper mode before it spawns the target, and never
spawns unrelated children. Consequently every direct or adopted child is owned by that run,
including an immediate double-fork that closes descriptors or becomes non-dumpable.

The supervisor tracks each owned process by PID plus procfs start time, prefers pidfds, and signals
individual identities only. It does not use raw process-group signals. The parent independently
bounds the supervisor, validates its fixed receipt, closes all pipe descriptors, and terminates the
supervisor-owned ancestry if the supervisor crashes, hangs, or returns malformed data. Separate
supervisors allow concurrent calls without exposing unrelated parent forks to ownership discovery.

Machine-readable records contain digests, counts, tool basenames, and versions only. Raw output,
environment values, absolute tool/cache paths, and user data are never serialized. Generated
Markdown is a deterministic rendering of those redacted records and is reviewed separately.
