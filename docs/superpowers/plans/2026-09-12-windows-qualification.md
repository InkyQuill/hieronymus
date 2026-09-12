# Windows native qualification plan

**Goal:** Execute the deferred Windows checks, fix reproduced failures, and retain exact local evidence without overstating desktop acceptance.

**Architecture:** Exercise the existing native implementations on disposable NTFS storage. Keep qualification logs and acquired tools under the ignored `qualification/.artifacts/windows-native-2026-09-12/` directory. Separate automated contracts from installed-artifact and interactive observations.

**Tech Stack:** Rust 1.96.0/MSVC, Bun 1.4.0, PowerShell 7, protobuf, Windows x64.

**Spec:** `docs/desktop-filesystem-qualification.md`, `docs/desktop-windows.md`, and `docs/desktop-qualification.md`.

## Execution

- [x] Read the Windows qualification docs and inspect the clean source checkout.
- [x] Locate MSVC/Rust and acquire local Bun/protobuf prerequisites.
- [x] Run release-script tests with PowerShell-expanded paths and frontend typecheck/test/build.
- [x] Reproduce `cargo test -p hieronymus --all-features --locked --lib private_file::tests`; investigate the exact failing operation before editing native behavior.
- [x] Run the filesystem checklist, Windows broker tests, selected-helper contract, and desktop helper tests; use each reproduced failure as a regression test and rerun it after a minimal fix.
- [x] Run the AGENTS.md Rust fmt/clippy/test/rustdoc gates and record any nonportable historical test blockers precisely.
- [x] Assemble and qualify a Windows payload using pinned model members from the Linux release; canonical CI archive provenance remains unavailable.
- [x] Execute authorized Scheduler and Explorer observations in the current account; retain fresh-logon and remaining appearance/host rows as unqualified.
- [x] Update Windows documentation with commands, host identity, results, log hashes, fixes, and outstanding evidence requirements.

No real book data or existing product registration is a fixture. A passing synthetic test does not qualify native manager or installed-model behavior. Local artifacts do not replace candidate CI provenance. No release publication is part of this work.
