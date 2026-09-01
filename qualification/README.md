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
the ignored `qualification/.artifacts/` root. `python -m tools.qualification.clean` is a dry run.
Pass `--apply` to remove only the bounded transient allowlist. Acquired models are retained
unless `--include-model` is also explicit. Cleanup refuses symlinks, mount crossings, special
files, repository/home roots, and the translation workspace.

Machine-readable records contain digests, counts, tool basenames, and versions only. Raw output,
environment values, absolute tool/cache paths, and user data are never serialized. Generated
Markdown is a deterministic rendering of those redacted records and is reviewed separately.
