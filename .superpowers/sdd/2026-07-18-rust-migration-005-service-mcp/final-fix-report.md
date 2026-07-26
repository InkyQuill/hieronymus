# Phase 005 final fix report

Starting head: `fe8f0beb1ff66f693cedd89545ba5079e62f17bd`

## Finding resolution

### A. Daemon dream cleanup lifecycle

Confirmed. The recurring daemon scheduler applied a one-second cleanup deadline, but manual web-admin and `StoreDreamRunner`/MCP instances used the unbounded `DreamService` default.

Resolution:

- `DAEMON_DREAM_CLEANUP_DEADLINE` is the single daemon policy and is applied to recurring, manual admin, MCP HTTP, and the stdio compatibility backend's daemon-owned dream path.
- Manual and MCP dream start and terminal transitions publish admin refresh events.
- A real MCP dream with a trigger that persistently rejects audit cleanup returns within the deadline, surfaces the audit error, and releases the cross-process lock.
- A real daemon process with a manual admin dream blocked in provider I/O exits within the SIGTERM bound despite persistent cleanup failure and releases the lock.

### B. Shipping admin dashboard contract

Confirmed. The Rust endpoint returned four ad-hoc counts, an incomplete short-term object, and only a dream `state`; it also relied solely on the manual-request atomic flag, so a background cycle appeared `IDLE`.

Resolution:

- The dashboard now returns the eight categories consumed by the existing console: series, crystals, lessons, short-term memories, sessions, dream runs, pending proposals, and audit events.
- Short-term status now includes state, thresholds, urgency, and drain counts/progress.
- Dream status now includes state, current phase, progress, run ID, cycle ID, owner, and start time.
- Status derives from authoritative SQLite run/phase rows plus the cross-process dream lock, so background and MCP owners report `WORKING`.
- Frontend `Record<string, unknown>` placeholders were replaced with the complete typed contract. `AdminDashboard.svelte` preserves the same fields and UI; it only removes defensive `String`/`Number` casts that previously hid missing backend fields. This strengthens rather than relaxes the shipping client contract.

### C. Production admin event publishers

Confirmed. `/ws/admin` could relay manually injected events, but no production mutation published one.

Resolution:

- `AdminNotifier` is a small cloneable bounded-broadcast abstraction; domain/core code remains transport-independent.
- Successful provider/catalog/cache, settings, admin, and MCP mutations publish refresh events.
- Manual, MCP, and background dream lifecycle transitions publish refresh events.
- Publications happen only after successful persistence/acceptance. Failed provider validation has a negative regression.
- The end-to-end regression connects to `/ws/admin`, performs a real provider HTTP mutation, and receives `{"type":"refresh"}`.

### D. `series_init` compatibility alias

Confirmed. The MCP alias created a series workspace and `.hieronymus.json`, while `series_create` did not.

Resolution: both MCP names now execute the identical registry operation and return the same contract. Workspace initialization remains a CLI-only boundary. The parity regression verifies the alias and absence of the filesystem side effect.

### E. Provider catalog concurrency

Confirmed. Independent requests could each load the same snapshot and overwrite one another during save.

Resolution:

- `AppState` owns one async mutation mutex.
- Provider upsert and delete hold it for the complete local load-modify-save critical section.
- Provider model/check operations read snapshots and perform external network I/O without this mutex.
- Concurrent independent upserts preserve all profiles; concurrent save/delete preserves the new profile without resurrecting the deleted profile.

### F. Override asset resource bound

Confirmed. Secure anchored `openat` traversal ended in unbounded `read_to_end`.

Resolution:

- Filesystem override assets are documented and capped at 16 MiB.
- Descriptor metadata rejects already-oversized regular files before allocation.
- `Read::take(limit + 1)` detects files that grow after metadata inspection.
- The existing descriptor-anchored, no-follow race protection remains intact.

## RED/GREEN evidence

Observed RED before production changes:

- dashboard literal contract: old four-count object differed from the eight-field contract;
- provider concurrency: only 1 of 16 independent upserts survived;
- event publication: no event was received after a successful provider save;
- `series_init`: behavior diverged and wrote `.hieronymus.json`;
- oversized asset: the complete 16 MiB + 1 byte file was returned;
- lifecycle premise: only the recurring construction site called `with_cleanup_deadline`.

Focused GREEN:

- `test_service`: 49 tests;
- `test_mcp`: 22 tests;
- `task5_process`: 8 tests;
- `test_dreaming`: 35 passed, 1 intentionally ignored process helper;
- frontend: two TypeScript configurations, 16 Vitest tests, production build, and Prettier check.

Full workspace GREEN:

- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`;
- `cargo test --workspace --all-features --locked`;
- `cargo doc --workspace --no-deps --all-features --locked`.

## Deferred final-audit notes

- Do not broaden this wave into missing-docs or `#[non_exhaustive]` cleanup; audit public API documentation and forward-compatibility separately.
- The existing supervisor invariant still treats an unexpected early `Ok(())` from a recurring worker as normal completion. Classify it as fatal when another recurring worker makes the generalized API worthwhile.
- The pre-existing dream cancellation test has a known timing-sensitive history. This wave did not weaken or ignore it; focused and full runs retain it.
