# Rust Runtime, Ownership, and Upgrade Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make daemon ownership, startup, maintenance, and release recovery obey ADRs 0009–0010 before wiring additional writers.

**Architecture:** Keep the existing synchronous transport. Introduce one OS-backed data-root guard and one bounded classifier shared by startup and maintenance; ordered upgrades and daemon-owned workers build on those boundaries.

**Tech Stack:** Rust 1.96 / edition 2024, rusqlite/SQLite FTS5, existing blocking daemon workers, Svelte 5, TypeScript, Bun 1.4.0; preserve Cargo.lock native pins.

**Spec:** [ADRs 0009–0010](../../adr/0009-runtime-topology-and-daemon-lifecycle.md), [upgrade design](../specs/2026-08-31-rust-database-upgrade-design.md), [daemon design](../specs/2026-08-31-rust-daemon-mcp-security-design.md). Also read [the remediation coordinator](2026-09-05-rust-port-remediation.md) and [the reconciled review](../../astra-report.md).

## Global Constraints

- “All domain mutations used by normal CLI, MCP, and frontend workflows go through the daemon.” (ADR 0009)
- “The first Rust cutover supports `x86_64-unknown-linux-gnu`.” (ADR 0013)
- Owner requirement (2026-09-05): working memory and semantic RAG are mandatory. ADR 0013’s FTS-only allowance is not an acceptable completion or release alternative for this program.
- “Only an authenticated user acting through the explicit rule-approval operation may transition `candidate` to `active`” (ADR 0011).
- “the separate CSRF token layer is waived.” (ADR 0012, 2026-09-03 amendment)
- MCP revision remains `2026-07-28`. Preserve frozen Python inputs; new ADR-backed Rust expectations are separate versioned fixtures.
- Preserve unrelated work, immutable backups, local plaintext credentials, and the single data-root layout. No Python runtime rollback, dependency-pin refresh, publication, or user-service changes during unit tests.
- Steps below specify planned code, not code already implemented. Existing types are referenced by source module; new cross-task interfaces are declared explicitly.

---

## File structure and execution boundary

Own the runtime and maintenance boundary, not the domain tool implementations. R1 and R2 precede every new daemon writer. R3 supplies schema v2 used by D4 and M3. R5 supplies supervised workers consumed by D5 and S2.

- **R1:** Reject invalid startup state and stop migrating config during reads — `crates/hieronymus/src/state_classifier.rs`, `crates/hieronymus/src/lib.rs`, `crates/hieronymus/src/db.rs`, `crates/hieronymus/src/provider_config.rs`, `crates/hieronymus/src/dream_config.rs`, `crates/hiero/src/daemon/mod.rs`, `crates/hieronymus/tests/startup_state.rs`
- **R2:** Enforce one owner across daemon, upgrade, and recovery — `crates/hieronymus/src/ownership.rs`, `crates/hieronymus/src/lib.rs`, `crates/hieronymus/src/upgrade.rs`, `crates/hiero/src/daemon/mod.rs`, `crates/hieronymus/tests/ownership.rs`, `crates/hiero/tests/daemon_lifecycle.rs`
- **R3:** Add ordered Rust schema upgrades and schema v2 durable work tables — `crates/hieronymus/src/schema_upgrade.rs`, `crates/hieronymus/migrations/002-durable-work.sql`, `crates/hieronymus/tests/fixtures/rust-v1.sql`, `crates/hieronymus/src/lib.rs`, `crates/hieronymus/src/db.rs`, `crates/hieronymus/src/upgrade.rs`, `crates/hieronymus/src/migrate.rs`, `crates/hiero/src/update.rs`, `crates/hieronymus/tests/rust_upgrade.rs`
- **R4:** Make update rollback and upgrade finalization reliable — `crates/hiero/src/update.rs`, `crates/hiero/src/service.rs`, `crates/hieronymus/src/upgrade.rs`, `crates/hiero/tests/update_flow.rs`, `crates/hieronymus/tests/upgrade_port.rs`
- **R5:** Own worker shutdown, stable credentials, and authenticated lifecycle commands — `crates/hiero/src/daemon/workers.rs`, `crates/hiero/src/lifecycle.rs`, `Cargo.toml`, `Cargo.lock`, `crates/hiero/src/lib.rs`, `crates/hiero/src/main.rs`, `crates/hiero/src/client.rs`, `crates/hiero/src/daemon/mod.rs`, `crates/hiero/src/daemon/discovery.rs`, `crates/hiero/src/daemon/rest/status.rs`, `crates/hiero/src/stdio.rs`, `crates/hiero/src/doctor.rs`, `crates/hiero/src/agent_hook.rs`, `crates/hiero/tests/runtime_shutdown.rs`

## Tasks

### Task R1: Reject invalid startup state and stop migrating config during reads

**Coverage:** Astra 15–16; ADR 0009 bounded classifier; ADR 0010 config preservation.

**Dependencies:** None.

**Files:**

- Create: `crates/hieronymus/src/state_classifier.rs`
- Modify: `crates/hieronymus/src/lib.rs`
- Modify: `crates/hieronymus/src/db.rs`
- Modify: `crates/hieronymus/src/provider_config.rs`
- Modify: `crates/hieronymus/src/dream_config.rs`
- Modify: `crates/hiero/src/daemon/mod.rs`
- Test: `crates/hieronymus/tests/startup_state.rs`

**Interfaces:** New state_classifier::classify(&HieronymusConfig) -> Result<StartupState, ClassifyError>; StartupState is Fresh or Current. ClassifyError carries a stable code and remediation command. Existing load_provider_catalog/load_dream_config keep their Result types but become read-only; only upgrade.rs owns conversion writes.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hieronymus::{data_root::HieronymusConfig, state_classifier::classify};
#[test]
fn invalid_config_is_rejected_without_rewriting_it() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let bytes = b"[not valid TOML\n";
    std::fs::write(config.dream_config_path(), bytes).unwrap();
    assert!(classify(&config).is_err());
    assert_eq!(std::fs::read(config.dream_config_path()).unwrap(), bytes);
    assert!(!config.daemon_discovery_path().exists());
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hieronymus --test startup_state --locked`
Expected before the change: new classifier unavailable; after adding the interface, malformed config and marker-only databases must still fail until the real guards exist.

- [ ] **Step 3: Implement the bounded change**

Classify config syntax and recognized legacy shapes with the existing read-only parsers and migrate config inventory. Missing configs may use current defaults; malformed, legacy, newer or unrecognized version markers are explicit errors. For a marked Rust database require the current mandatory tables/columns and consistent user_version/meta markers using sqlite_master and PRAGMA table_info, not integrity_check or data scans. Fresh creation must be one transaction and publish its marker last. Remove writes from both normal load functions; legacy provider/workflow formats return config_migration_required. Startup calls classify before open_migrated, binding or writing credentials/discovery. Dry-run uses the same classifier but continues into its explicit full rehearsal for migration-required states.

```rust
#[derive(Debug, PartialEq, Eq)]
pub enum StartupState { Fresh, Current }
// Existing parser errors are mapped to a stable config error; never save here.
pub fn load_dream_config(config: &HieronymusConfig) -> Result<DreamConfig, DreamConfigError> {
    resolve_dream_config_readonly(config)
}
// Before this return, the readonly parser must explicitly reject legacy shapes,
// rather than silently normalizing them. Conversion stays in upgrade.rs.
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hieronymus --test startup_state --locked`
Expected after the change: malformed config, unsupported config shape, absent required tables, and mixed markers reject; fresh/current roots still start.

Add cases for a supported marker with no domain tables; a current database plus legacy provider blocks; ordinary loads/GET routes leaving all config bytes unchanged; a fresh-schema SQL failure leaving no version marker. Run cargo test -p hieronymus --test config_port --test provider_config_port --test upgrade_port --locked and cargo test -p hiero --test daemon_lifecycle --locked.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hieronymus/src/state_classifier.rs crates/hieronymus/src/lib.rs crates/hieronymus/src/db.rs crates/hieronymus/src/provider_config.rs crates/hieronymus/src/dream_config.rs crates/hiero/src/daemon/mod.rs crates/hieronymus/tests/startup_state.rs
git commit -m "fix: classify startup state and make config reads non-mutating"
```

### Task R2: Enforce one owner across daemon, upgrade, and recovery

**Coverage:** Astra 9; ownership part of 14–16; ADR 0009.

**Dependencies:** R1.

**Files:**

- Create: `crates/hieronymus/src/ownership.rs`
- Modify: `crates/hieronymus/src/lib.rs`
- Modify: `crates/hieronymus/src/upgrade.rs`
- Modify: `crates/hiero/src/daemon/mod.rs`
- Test: `crates/hieronymus/tests/ownership.rs`
- Modify: `crates/hiero/tests/daemon_lifecycle.rs`

**Interfaces:** RootOwnership::acquire(&HieronymusConfig, &str) -> std::io::Result<RootOwnership>. Guard owns an open File for data-root/.owner.lock; Drop unlocks. Never unlink the lock inode. Daemon stores the guard for its full lifetime; run_upgrade/run_recovery acquire it before any preflight copying/staging.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hieronymus::{data_root::HieronymusConfig, ownership::RootOwnership};
#[test]
fn different_handles_cannot_own_the_same_root() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let first = RootOwnership::acquire(&config, "daemon").unwrap();
    assert!(RootOwnership::acquire(&config, "migrate").is_err());
    drop(first);
    assert!(RootOwnership::acquire(&config, "recover").is_ok());
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hieronymus --test ownership --locked`
Expected before the change: ownership module absent or a second guard incorrectly succeeds.

- [ ] **Step 3: Implement the bounded change**

Use std::fs::File::try_lock once; keep the guard open. Write bounded owner metadata only after acquiring the lock; diagnostics may show role/PID, but PID checks must never grant ownership. Replace upgrade.rs's PID/create-file lock, including its special same-process reacquire behavior. Test injection must release real guards when unwinding; subprocess crash tests establish OS lock release. Acquire before read/classify and recheck journal under ownership. Do not call an already-locking upgrade recursively during recovery; pass an internal guard reference to locked helpers.

```rust
pub struct RootOwnership { file: std::fs::File }
impl RootOwnership {
    pub fn acquire(config: &HieronymusConfig, role: &str) -> std::io::Result<Self> {
        use std::io::{Seek, Write};
        std::fs::create_dir_all(config.data_root())?;
        let mut file = std::fs::OpenOptions::new()
            .read(true).write(true).create(true).truncate(false)
            .open(config.data_root().join(".owner.lock"))?;
        file.try_lock().map_err(std::io::Error::other)?;
        file.set_len(0)?;
        file.rewind()?;
        write!(file, "{} {}\n", std::process::id(), role)?;
        file.sync_all()?;
        Ok(Self { file })
    }
}
impl Drop for RootOwnership {
    fn drop(&mut self) { let _ = self.file.unlock(); }
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hieronymus --test ownership --locked`
Expected after the change: second owner refuses, owner exit releases the OS lock, independent roots work.

Add separate-process tests: two daemon ports on one root; daemon start concurrent with migrate before journal creation; recover racing daemon start; SIGKILL then reacquire. Confirm no second daemon overwrites token/discovery. Read-only doctor remains lock-free and non-mutating.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hieronymus/src/ownership.rs crates/hieronymus/src/lib.rs crates/hieronymus/src/upgrade.rs crates/hiero/src/daemon/mod.rs crates/hieronymus/tests/ownership.rs crates/hiero/tests/daemon_lifecycle.rs
git commit -m "fix: enforce shared data-root ownership"
```

### Task R3: Add ordered Rust schema upgrades and schema v2 durable work tables

**Coverage:** Sonnet 3.3; foundation for Sonnet 3.2 and ADR 0011 audited rule actions.

**Dependencies:** R1–R2.

**Files:**

- Create: `crates/hieronymus/src/schema_upgrade.rs`
- Create: `crates/hieronymus/migrations/002-durable-work.sql`
- Create: `crates/hieronymus/tests/fixtures/rust-v1.sql`
- Modify: `crates/hieronymus/src/lib.rs`
- Modify: `crates/hieronymus/src/db.rs`
- Modify: `crates/hieronymus/src/upgrade.rs`
- Modify: `crates/hieronymus/src/migrate.rs`
- Modify: `crates/hiero/src/update.rs`
- Test: `crates/hieronymus/tests/rust_upgrade.rs`

**Interfaces:** schema_upgrade::apply_steps(&rusqlite::Transaction<'_>, i64, i64) -> rusqlite::Result<()>; production steps 1→2. Existing run_upgrade handles Python/current/older Rust using its same ownership, backups, verification and journal. No converter commits. New tables: dream_link_batches, dream_link_members, dream_link_pairs, term_rule_actions. Batch source IDs are durable snapshot identifiers without live-row foreign keys: deleting a source record must not erase pair progress or block normal deletion; D4 revalidates them and records skipped tombstones. Internal batch references retain foreign keys.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hieronymus::schema_upgrade::apply_steps;
#[test]
fn failed_upgrade_does_not_publish_the_new_version() {
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(include_str!("fixtures/rust-v1.sql")).unwrap();
    {
        let tx = db.transaction().unwrap();
        apply_steps(&tx, 1, 2).unwrap();
        // Dropping the transaction simulates failure before the shared commit.
    }
    let version: i64 = db.query_row(
        "select schema_version from hieronymus_meta", [], |r| r.get(0)).unwrap();
    assert_eq!(version, 1);
    let tables: i64 = db.query_row(
        "select count(*) from sqlite_master where name='dream_link_pairs'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(tables, 0);
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hieronymus --test rust_upgrade --locked`
Expected before the change: runner absent; rollback/second-upgrade tests expose unconditional already-complete handling.

- [ ] **Step 3: Implement the bounded change**

Capture schema-v1 fixture from the reviewed global.sql + terminology.sql plus the v1 meta/user_version before changing fresh creation. Fresh v2 creation applies the same v1 baseline and ordered 002 step in one transaction. Preserve IDs and data; never alter an already released SQL file. A complete journal is a no-op only when its target equals the requested current target and the database agrees; otherwise preserve its receipt/backup and begin a new numbered attempt. Back up current Rust databases with the existing durable protocol; skip Python-only terminology converters and verify all relevant foreign keys/FTS. Recovery classifies copied backups and supports both Python and older Rust. Update schema_gate compares candidate support against the actual disk version, not only the updater's own NewerSchema label.

```sql
create table dream_link_batches (
  id integer primary key, session_id integer not null,
  created_cycle integer not null, completed_cycle integer
);
create table dream_link_members (
  batch_id integer not null references dream_link_batches(id),
  activation_id integer not null unique,
  primary key(batch_id, activation_id)
);
create table dream_link_pairs (
  batch_id integer not null references dream_link_batches(id),
  left_id integer not null,
  right_id integer not null,
  status text not null default 'queued' check(status in ('queued','applied','skipped')),
  applied_cycle integer, result_json text,
  primary key(batch_id,left_id,right_id), check(left_id < right_id)
);
create table term_rule_actions (
  id integer primary key, idempotency_key text not null unique,
  rule_id integer not null references term_rules(id),
  actor text not null, reason text not null, action text not null,
  prior_revision integer not null, resulting_revision integer not null,
  request_json text not null, result_json text not null, created_at text not null
);
update hieronymus_meta set schema_version=2;
pragma user_version=2;
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hieronymus --test rust_upgrade --locked`
Expected after the change: v1→v2 is transactional, rerun is a no-op, a completed earlier cutover does not block a later target.

Test copied v1 data row preservation; failed SQL rollback; foreign-key/integrity failures; Python→v2 and v1→v2 recovery; a complete v1 journal followed by v2 upgrade; candidate supports v2 while updater knows v1; reject an actual disk version above candidate support. No writes on daemon open of old Rust state.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hieronymus/src/schema_upgrade.rs crates/hieronymus/migrations/002-durable-work.sql crates/hieronymus/tests/fixtures/rust-v1.sql crates/hieronymus/src/lib.rs crates/hieronymus/src/db.rs crates/hieronymus/src/upgrade.rs crates/hieronymus/src/migrate.rs crates/hiero/src/update.rs crates/hieronymus/tests/rust_upgrade.rs
git commit -m "feat: add ordered Rust schema upgrades"
```

### Task R4: Make update rollback and upgrade finalization reliable

**Coverage:** Astra 10, receipt follow-up; Sonnet 3.3 repeat upgrades.

**Dependencies:** R2–R3.

**Files:**

- Modify: `crates/hiero/src/update.rs`
- Modify: `crates/hiero/src/service.rs`
- Modify: `crates/hieronymus/src/upgrade.rs`
- Modify: `crates/hiero/tests/update_flow.rs`
- Modify: `crates/hieronymus/tests/upgrade_port.rs`

**Interfaces:** Add update::accepted_doctor_exit(Option<i32>) -> Result<bool, String> (false healthy, true accepted degraded). Add service::ServiceManager trait with stop(), reload(), start() -> Result<(), ServiceError>; Systemd implementation and stateful fake. Rollback returns Result<(), UpdateError>, not unit.

- [ ] **Step 1: Add the failing regression**

Place this unit regression inside update.rs's tests module; add process cases in update_flow.rs using its existing fake-release helpers.

```rust
#[test]
fn unexpected_doctor_exit_is_not_healthy() {
    assert!(super::accepted_doctor_exit(Some(42)).is_err());
    assert!(super::accepted_doctor_exit(None).is_err());
    assert_eq!(super::accepted_doctor_exit(Some(0)).unwrap(), false);
    assert_eq!(super::accepted_doctor_exit(Some(1)).unwrap(), true);
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hiero update --locked`
Expected before the change: new helper absent; a fake release exiting 42 is currently kept.

- [ ] **Step 3: Implement the bounded change**

Allow only 0/1; doctor exit1 is acceptable only for a non-semantic warning after authenticated expected-instance readiness and S2 require_semantic_ready succeeds. A missing/disarmed semantic lane always rejects the candidate; code1 alone never authorizes activation. Keep the previous link/unit snapshot before stop. Every fallible operation after stop/switch, including command spawn errors, enters the same rollback state machine: stop candidate, restore links/unit, reload manager, restart previous if it was running, verify readiness. Preserve candidate and previous artifacts if recovery fails; return both original and rollback errors without claiming restoration. Write/recover receipt before the journal's terminal complete transition; an older complete journal lacking a receipt is finalized idempotently from stored checksums.

```rust
pub fn accepted_doctor_exit(code: Option<i32>) -> Result<bool, String> {
    match code {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        other => Err(format!("candidate doctor failed: {other:?}")),
    }
}
// Rollback is an ordered operation, and each step propagates its error:
// manager.stop()?; restore_links_and_unit()?; manager.reload()?;
// if previous_was_running { manager.start()?; verify_previous_instance()?; }
// Keep the previous version until this sequence is verified.
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hiero update --locked`
Expected after the change: unexpected exits/spawn errors recover; failed rollback is reported; no complete journal lacks a recoverable receipt.

The rollback sketch's restore_links_and_unit and verify_previous_instance are local helpers to define in update.rs: each returns Result<(), UpdateError>, takes the saved snapshot/expected DiscoveryRecord, and never deletes recovery artifacts on error. Test exact manager call order, slow readiness, failed stop/reload/start, old-running versus old-stopped, receipt write failure and resumed finalization. Run cargo test -p hiero --test update_flow --locked and cargo test -p hieronymus --test upgrade_port --locked.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/update.rs crates/hiero/src/service.rs crates/hieronymus/src/upgrade.rs crates/hiero/tests/update_flow.rs crates/hieronymus/tests/upgrade_port.rs
git commit -m "fix: make update rollback and upgrade receipts recoverable"
```

### Task R5: Own worker shutdown, stable credentials, and authenticated lifecycle commands

**Coverage:** Astra 9/11 and shutdown/discovery follow-ups; Sonnet 2.4 and SIGTERM assumption.

**Dependencies:** R1–R2; R4 uses this task's readiness probe when integrated.

**Files:**

- Create: `crates/hiero/src/daemon/workers.rs`
- Create: `crates/hiero/src/lifecycle.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/hiero/src/lib.rs`
- Modify: `crates/hiero/src/main.rs`
- Modify: `crates/hiero/src/client.rs`
- Modify: `crates/hiero/src/daemon/mod.rs`
- Modify: `crates/hiero/src/daemon/discovery.rs`
- Modify: `crates/hiero/src/daemon/rest/status.rs`
- Modify: `crates/hiero/src/stdio.rs`
- Modify: `crates/hiero/src/doctor.rs`
- Modify: `crates/hiero/src/agent_hook.rs`
- Test: `crates/hiero/tests/runtime_shutdown.rs`

**Interfaces:** WorkerGroup::new(Arc<AtomicBool>) -> WorkerGroup; spawn(&self, Box<dyn FnOnce(Arc<AtomicBool>) + Send>) -> Result<(), String>; stop_and_join(&self) -> Result<(), String>. lifecycle::connect(&HieronymusConfig, bool) -> Result<DaemonClient, ClientError>; DaemonClient::post(&self,&str,&Value) -> Result<Value,ClientError>. client::request_json adds method parameter while post_json remains a wrapper. Authenticated status DTO adds instance_id, pid, version, protocol_revision.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hiero::daemon::{Daemon, DaemonOptions};
#[test]
fn restart_reuses_installation_token() {
    let root = tempfile::tempdir().unwrap();
    let options = DaemonOptions { data_root: Some(root.path().into()), port: 0,
        ..Default::default() };
    let first = Daemon::start(&options).unwrap();
    let token = first.bearer().expose_secret().clone();
    first.shutdown().unwrap();
    let second = Daemon::start(&options).unwrap();
    assert_eq!(second.bearer().expose_secret(), &token);
    second.shutdown().unwrap();
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hiero --test runtime_shutdown --locked`
Expected before the change: restart changes the token; shutdown returns while detached writers still run.

- [ ] **Step 3: Implement the bounded change**

Load existing protected token under ownership; create with 0600 only when absent, reject empty/insecure credentials with repair guidance. WorkerGroup owns connection/dream/index handles and a stop flag: stop admission, close/wake socket reads, signal cancellation, join writers, then remove matching discovery and release RootOwnership. All blocking I/O retains bounded timeouts. On a shutdown deadline failure keep ownership until process exit; never let another owner enter while a writer is live. Enable ctrlc's termination feature so SIGTERM reaches the same path. Add start/stop/restart/status CLI commands; stop posts /shutdown to authenticated discovered foreground or managed daemon; start installs/starts the per-user service. Preserve service subcommands. Stdio may autostart via service integration; repair stale discovery only after authenticated instance comparison fails. Doctor/hook use the same probe, without starting or deleting anything.

```rust
// Cargo.toml: ctrlc = { version = "3", features = ["termination"] }
// WorkerGroup stores Mutex<Vec<JoinHandle<()>>> and the shared stop flag.
// A shutdown first stops admission; joining is outside the accept-loop thread.
pub fn stop_and_join(&self) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    self.stop.store(true, Ordering::Release);
    let handles = std::mem::take(&mut *self.handles.lock().map_err(|_| "worker lock poisoned")?);
    for handle in handles {
        handle.join().map_err(|_| "worker panicked")?;
    }
    Ok(())
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hiero --test runtime_shutdown --locked`
Expected after the change: restart keeps token; SIGINT/SIGTERM/RPC shutdown drains writers and removes only its own discovery; spoof TCP/PID is not available.

Define WorkerGroup fields as stop: Arc<AtomicBool>, handles: Mutex<Vec<JoinHandle<()>>>; guard spawn with admission under that mutex. Process-test SIGTERM, two successive shutdowns, a stalled socket, a worker finishing a transaction, stale-port reuse by a dummy listener, protocol/instance mismatch and unmanaged stop. Expose no token in status/errors. Verify generated integrations read discovery and never hard-code port 9768.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/daemon/workers.rs crates/hiero/src/lifecycle.rs Cargo.toml Cargo.lock crates/hiero/src/lib.rs crates/hiero/src/main.rs crates/hiero/src/client.rs crates/hiero/src/daemon/mod.rs crates/hiero/src/daemon/discovery.rs crates/hiero/src/daemon/rest/status.rs crates/hiero/src/stdio.rs crates/hiero/src/doctor.rs crates/hiero/src/agent_hook.rs crates/hiero/tests/runtime_shutdown.rs
git commit -m "fix: supervise daemon workers and implement authenticated lifecycle"
```

## Plan self-review

Coverage is mapped in the coordinator. Every task above has a regression, implementation sketch, explicit interfaces, and a focused verification command. Execute prerequisites first; snippets using newly introduced APIs intentionally fail to compile before those APIs land. Do not interpret a passing compile as the behavioral green step.

