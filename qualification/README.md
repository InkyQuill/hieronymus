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

Every process run gets a fresh kernel PID lifetime boundary. The fixed canonical
`/usr/bin/unshare` executable creates a user namespace plus a PID namespace with a private procfs;
its fixed options map the caller to namespace root, fork a namespace init, and use
`--kill-child=SIGKILL`. The namespace init is a single-purpose Python supervisor. Linux kills every
remaining member of a PID namespace when its init exits, so a supervisor crash, malformed reply,
hang, timeout, `setsid`, double fork, non-dumpable child, or fork concurrent with shutdown cannot
leave a qualification process behind. The parent-death signal on the unshare wrapper closes the
same boundary if the Python caller is killed. Linux hosts without this exact boundary fail before
the target is spawned; there is no process-group or procfs-snapshot fallback.

A tiny launcher sets `RLIMIT_CORE=(0,0)` before it can read configuration, arms the parent-death
signal, and then replaces itself with unshare. The wrapper, namespace supervisor, target, and every
descendant inherit the zero core limit. The caller passes the already sanitized argv, canonical
working directory, exact-string environment, and exact positive integer time bounds only through
inherited anonymous pipes; none of those values appear in command lines, files, logs, errors, or
receipts. The supervisor still tracks and reaps normal children by PID plus procfs start time and
pidfd, while the kernel boundary is authoritative for abnormal termination and the final fork race.
The parent validates the fixed receipt and closes every pipe descriptor before returning. Separate
namespaces allow concurrent calls without exposing or signalling unrelated parent processes.

Machine-readable records contain digests, counts, tool basenames, and versions only. Raw output,
environment values, absolute tool/cache paths, and user data are never serialized. Generated
Markdown is a deterministic rendering of those redacted records and is reviewed separately.
