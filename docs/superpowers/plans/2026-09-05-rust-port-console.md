# Rust web console completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Launch an authenticated console from the CLI and make every advertised view and action work against the daemon.

**Architecture:** The CLI exchanges its installation credential for a short-lived launch grant, and the browser exchanges that grant for the existing session cookie. Thin REST handlers share domain operations with Application; typed Svelte dialogs expose the required action inputs.

**Tech Stack:** Rust 1.96 / edition 2024, rusqlite/SQLite FTS5, existing blocking daemon workers, Svelte 5, TypeScript, Bun 1.4.0; preserve Cargo.lock native pins.

**Spec:** ADRs 0001, 0007, 0011, 0012 (including the CSRF waiver), and 0014; Astra 2–4 and 12. Also read [the remediation coordinator](2026-09-05-rust-port-remediation.md) and [the reconciled review](../../astra-report.md).

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

`console.rs` owns CLI launch; `bootstrap.ts` owns the one-time browser exchange. `application/admin.rs` owns admin projections/actions. Keep authentication in REST and provider checks in their existing module.

- **W1:** Bootstrap console sessions and correct browser Origin checks — `crates/hiero/src/console.rs`, `crates/hiero/src/lib.rs`, `crates/hiero/src/main.rs`, `crates/hiero/src/daemon/rest/mod.rs`, `crates/hiero/src/daemon/sessions.rs`, `frontend/src/web/lib/bootstrap.ts`, `frontend/src/web/main.ts`, `frontend/src/web/lib/bootstrap.test.ts`, `crates/hiero/tests/console_auth.rs`, `compatibility/README.md`
- **W2:** Implement all ten admin views as domain projections — `crates/hiero/src/application/admin.rs`, `crates/hiero/src/application/mod.rs`, `crates/hiero/src/daemon/rest/admin.rs`, `frontend/src/web/lib/types.ts`, `frontend/src/web/components/MemoryViews.svelte`, `crates/hiero/tests/admin_views.rs`, `frontend/src/web/components/MemoryViews.test.ts`
- **W3:** Implement typed admin actions and remove production provider fixtures — `crates/hiero/src/application/admin.rs`, `crates/hiero/src/daemon/rest/admin.rs`, `crates/hiero/src/daemon/rest/providers.rs`, `frontend/src/web/lib/api.ts`, `frontend/src/web/lib/types.ts`, `frontend/src/web/components/MemoryViews.svelte`, `frontend/src/web/components/ActionDialog.svelte`, `frontend/src/web/components/ActionDialog.test.ts`, `crates/hiero/tests/admin_actions.rs`, `crates/hiero/tests/provider_checks.rs`
- **W4:** Resume and deduplicate admin events without stale refreshes — `frontend/src/web/lib/admin-events.svelte.ts`, `frontend/src/web/components/MemoryViews.svelte`, `frontend/src/web/lib/admin-events.test.ts`, `crates/hiero/src/daemon/events.rs`, `crates/hiero/tests/daemon_ws_routes.rs`

## Tasks

### Task W1: Bootstrap console sessions and correct browser Origin checks

**Coverage:** Astra 2–3; Sonnet's functional-console claim is not verified by fixtures.

**Dependencies:** R5.

**Files:**

- Create: `crates/hiero/src/console.rs`
- Modify: `crates/hiero/src/lib.rs`
- Modify: `crates/hiero/src/main.rs`
- Modify: `crates/hiero/src/daemon/rest/mod.rs`
- Modify: `crates/hiero/src/daemon/sessions.rs`
- Create: `frontend/src/web/lib/bootstrap.ts`
- Modify: `frontend/src/web/main.ts`
- Test: `frontend/src/web/lib/bootstrap.test.ts`
- Test: `crates/hiero/tests/console_auth.rs`
- Modify: `compatibility/README.md`

**Interfaces:** Produce `console::launch(config: &HieronymusConfig, page: &str) -> Result<(),String>` for `hiero admin` and `hiero config`; consumes R5 lifecycle::connect and DaemonClient::post. Produce TS `takeLaunchGrant(location: Pick<Location,'hash'|'pathname'|'search'>, history: Pick<History,'replaceState'>): string | null` and `bootstrapSession(): Promise<void>`. Browser calls existing POST /auth/launch-grant/exchange with JSON grant; App mounts only after success.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```typescript
import { expect, test, vi } from 'vitest';
import { takeLaunchGrant } from './bootstrap';

test('consumes grant from history before any network request', () => {
  const history = { replaceState: vi.fn() };
  expect(takeLaunchGrant(
    { hash: '#launch_grant=abc', pathname: '/admin', search: '' },
    history,
  )).toBe('abc');
  expect(history.replaceState).toHaveBeenCalledWith(null, '', '/admin');
});
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cd frontend && bun run test src/web/lib/bootstrap.test.ts`
Expected before the change: bootstrap module is absent; frontend requests protected data without obtaining a session.

- [ ] **Step 3: Implement the bounded change**

Use the existing 60-second, single-use grant. CLI obtains it with bearer-authenticated POST, passes it to xdg-open as a URL fragment, and never prints/logs the URL. The fragment is scrubbed synchronously before awaiting fetch or mounting App; no persistent grant storage. This is a specific design choice: ADR 0012 prohibits query-string secrets; fragment avoidance in Astra was a recommendation, not an accepted ADR requirement. Update sessions.rs's stronger comment to match this reviewed transport. Preserve HttpOnly/SameSite session cookies, Host validation, grant replay/expiry checks and daemon-lifetime sessions. Accept a missing Origin only for authenticated safe browser reads (GET/HEAD); reject an explicit foreign Origin on reads and require the exact allowed Origin on mutations, grant exchange and WS upgrade. No CSRF token restoration. Reconcile changed expectations in new Rust fixtures without editing frozen snapshot bytes.

```typescript
export function takeLaunchGrant(
  location: Pick<Location, 'hash' | 'pathname' | 'search'>,
  history: Pick<History, 'replaceState'>,
): string | null {
  const grant = new URLSearchParams(location.hash.slice(1)).get('launch_grant');
  if (grant !== null) {
    history.replaceState(null, '', location.pathname + location.search);
  }
  return grant;
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cd frontend && bun run test src/web/lib/bootstrap.test.ts`
Expected after the change: grant is removed immediately, exchanged once, then authenticated GETs succeed without Origin.

Run `cargo test -p hiero --test console_auth`. Add live daemon cases for GET without Origin=200 with session, missing session=401, foreign Origin=403, mutating request without Origin=403, correct Origin success, expired/replayed grant rejection, restart session expiry. Browser exchange errors show a relaunch instruction, not an empty dashboard. Test CLI page selection and opener failure without exposing grant/bearer in output.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/console.rs crates/hiero/src/lib.rs crates/hiero/src/main.rs crates/hiero/src/daemon/rest/mod.rs crates/hiero/src/daemon/sessions.rs frontend/src/web/lib/bootstrap.ts frontend/src/web/main.ts frontend/src/web/lib/bootstrap.test.ts crates/hiero/tests/console_auth.rs compatibility/README.md
git commit -m "fix: bootstrap authenticated web console"
```

### Task W2: Implement all ten admin views as domain projections

**Coverage:** Astra 4; ADR 0014 feature completeness.

**Dependencies:** M1–M4, W1.

**Files:**

- Create: `crates/hiero/src/application/admin.rs`
- Modify: `crates/hiero/src/application/mod.rs`
- Modify: `crates/hiero/src/daemon/rest/admin.rs`
- Modify: `frontend/src/web/lib/types.ts`
- Modify: `frontend/src/web/components/MemoryViews.svelte`
- Test: `crates/hiero/tests/admin_views.rs`
- Test: `frontend/src/web/components/MemoryViews.test.ts`

**Interfaces:** Produce `admin::VIEW_NAMES: [&str;10]` and `admin::snapshot(config: &HieronymusConfig, view: &str, query: &serde_json::Value) -> Result<Value,AppError>`. JSON projection retains existing snapshot keys/row identifiers from Python admin_models.py; paging/filter fields are typed in frontend lib/types.ts. Domain queries are read-only and share context filtering with Application.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hiero::application::admin::{snapshot, VIEW_NAMES};
use hieronymus::data_root::HieronymusConfig;
use serde_json::json;
#[test]
fn every_advertised_view_accepts_an_empty_database() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let _app = hiero::application::Application::open(&config).unwrap();
    assert_eq!(VIEW_NAMES.len(), 10);
    for name in VIEW_NAMES {
        let result = snapshot(&config, name, &json!({}));
        assert!(result.is_ok(), "{name}: {result:?}");
    }
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hiero --test admin_views`
Expected before the change: only Crystals and Dream Runs currently resolve; other advertised names fail.

- [ ] **Step 3: Implement the bounded change**

Implement Concepts, Renderings, Crystals, Lessons, Short-Term Memory, Short-Term Sessions, Dream Runs, Proposals, Dream Audits, and Audit Log using Python admin.py/admin_models.py as the non-ADR-conflicting result reference. Query real concept/facet/session/audit stores and preserve stable IDs, total counts, filters, detail records, and empty-state envelopes. Don't return an empty static array for an unimplemented view. Use bounded pagination and consistent series/story scopes. Surface query errors through AppError without exposing secrets. Frontend selects each real view, displays the typed columns and details, and preserves current selection across refresh by stable ID.

```rust
pub const VIEW_NAMES: [&str; 10] = [
    "Concepts", "Renderings", "Crystals", "Lessons", "Short-Term Memory",
    "Short-Term Sessions", "Dream Runs", "Proposals", "Dream Audits", "Audit Log",
];
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hiero --test admin_views`
Expected after the change: all ten views return valid projections for empty and seeded data, with matching frontend renderers.

Add one seeded visible record and a foreign-context record per view; verify paging and filters do not leak the foreign record. Frontend tests must select all ten views and show actual returned row content. Run `cd frontend && bun run test src/web/components/MemoryViews.test.ts && bun run typecheck`.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/application/admin.rs crates/hiero/src/application/mod.rs crates/hiero/src/daemon/rest/admin.rs frontend/src/web/lib/types.ts frontend/src/web/components/MemoryViews.svelte crates/hiero/tests/admin_views.rs frontend/src/web/components/MemoryViews.test.ts
git commit -m "feat: complete admin memory projections"
```

### Task W3: Implement typed admin actions and remove production provider fixtures

**Coverage:** Astra 4/12; Sonnet CSRF fixture documentation; Dream and rule lifecycle reuse.

**Dependencies:** W2, M3, D5.

**Files:**

- Modify: `crates/hiero/src/application/admin.rs`
- Modify: `crates/hiero/src/daemon/rest/admin.rs`
- Modify: `crates/hiero/src/daemon/rest/providers.rs`
- Modify: `frontend/src/web/lib/api.ts`
- Modify: `frontend/src/web/lib/types.ts`
- Modify: `frontend/src/web/components/MemoryViews.svelte`
- Create: `frontend/src/web/components/ActionDialog.svelte`
- Test: `frontend/src/web/components/ActionDialog.test.ts`
- Test: `crates/hiero/tests/admin_actions.rs`
- Test: `crates/hiero/tests/provider_checks.rs`

**Interfaces:** Produce `admin::validate_action_request(action: &str, args: &Value) -> Result<(),AppError>` and `admin::run_action(app: &Application, actor: &str, action: &str, args: &Value) -> Result<Value,AppError>`. Requests use typed per-action DTOs; destructive actions require confirmed=true. CLI/browser actors come from auth, not JSON. D5 provides run_manual_dreaming; M3 provides explicit rule actions.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```rust
use hiero::application::admin::validate_action_request;
use serde_json::json;
#[test]
fn split_requires_content_for_both_resulting_crystals() {
    assert!(validate_action_request("split_crystal", &json!({
        "id": 7, "confirmed": true
    })).is_err());
    assert!(validate_action_request("split_crystal", &json!({
        "id": 7, "confirmed": true, "parts": ["First memory", "Second memory"]
    })).is_ok());
}
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cargo test -p hiero --test admin_actions --test provider_checks`
Expected before the change: most action routes are missing, and fixture hostnames produce synthetic provider success in production.

- [ ] **Step 3: Implement the bounded change**

Implement add_memory(text,context/session), edit_memory(id,text), delete_selected(ids), merge_selected(ids,text), split_crystal(id,parts), reinforce_crystal(id), decay_crystal(id), approve_proposal(id,reason), reject_proposal(id,reason), inspect_provenance(id), inspect_recall_reasons(recall_id,id), run_manual_dreaming(all), and review_dream_output(run_id). Preserve frozen field names where defined; version additional fields explicitly. All actions validate IDs/context, use transactions with audit, and refresh affected projections. Rule approval/archive/replacement must call M3 instead of editing a crystal projection; deletion/merge/split of active rule projections must reject and direct the user to the explicit rule action. Dialogs gather real content and reason rather than sending only an ID. Remove `.invalid` success branches: production always invokes actual provider transport; tests inject ProviderTransport through the existing seam. Never report success merely because a hostname resembles a fixture.

```rust
pub fn validate_action_request(action: &str, args: &serde_json::Value) -> Result<(), AppError> {
    if action == "split_crystal" {
        let parts = args.get("parts").and_then(serde_json::Value::as_array)
            .ok_or_else(|| AppError::Invalid("parts must be an array".into()))?;
        if parts.len() < 2 || parts.iter().any(|part|
            part.as_str().is_none_or(|text| text.trim().is_empty())) {
            return Err(AppError::Invalid("split requires at least two nonempty parts".into()));
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cargo test -p hiero --test admin_actions --test provider_checks`
Expected after the change: each action performs and reports its domain effect; provider checks exercise real transport or return a real error.

Use a table of all 13 action names with success and invalid-input cases. Verify failed actions leave DB/audit consistent. Test protected rule operations, duplicate IDs, missing records and cancelled confirmation. Frontend tests enter draft text/parts/reason and assert request bodies plus accessible labels/focus. Inject a mock transport failure for a configured `.invalid` provider and assert error, never synthetic models. Run frontend tests/typecheck and the Rust affected modules.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add crates/hiero/src/application/admin.rs crates/hiero/src/daemon/rest/admin.rs crates/hiero/src/daemon/rest/providers.rs frontend/src/web/lib/api.ts frontend/src/web/lib/types.ts frontend/src/web/components/MemoryViews.svelte frontend/src/web/components/ActionDialog.svelte frontend/src/web/components/ActionDialog.test.ts crates/hiero/tests/admin_actions.rs crates/hiero/tests/provider_checks.rs
git commit -m "feat: complete admin actions and real provider checks"
```

### Task W4: Resume and deduplicate admin events without stale refreshes

**Coverage:** Sonnet event-delivery follow-up; observable long-running Dream/index work.

**Dependencies:** W2, D5, S2.

**Files:**

- Modify: `frontend/src/web/lib/admin-events.svelte.ts`
- Modify: `frontend/src/web/components/MemoryViews.svelte`
- Test: `frontend/src/web/lib/admin-events.test.ts`
- Modify: `crates/hiero/src/daemon/events.rs`
- Test: `crates/hiero/tests/daemon_ws_routes.rs`

**Interfaces:** Produce TS `acceptEvent(last: number, value: unknown): { last: number; refresh: boolean }`; consume version=1, event_id, event_type, payload wire envelope. connectAdminEvents retains its existing callback/cleanup signature and keeps cursor for the current daemon session only.

- [ ] **Step 1: Add the failing regression**

Place this test in the listed test file. Imports in the snippet are part of the test.

```typescript
import { expect, test } from 'vitest';
import { acceptEvent } from './admin-events.svelte';

test('does not refresh twice for replay/live duplicate delivery', () => {
  const event = { version: 1, event_id: 9, event_type: 'dream_phase_completed', payload: {} };
  expect(acceptEvent(8, event)).toEqual({ last: 9, refresh: true });
  expect(acceptEvent(9, event)).toEqual({ last: 9, refresh: false });
});
```

- [ ] **Step 2: Run the regression and confirm the failure is behavioral**

Run: `cd frontend && bun run test src/web/lib/admin-events.test.ts`
Expected before the change: client refreshes unconditionally and never sends a resume cursor.

- [ ] **Step 3: Implement the bounded change**

Validate the envelope, send resume_from_event_id on reconnect, and discard replay/live duplicates. snapshot_refresh must refresh even if its cursor is old; reset cursor when a new authenticated daemon session is established. Coalesce refresh callbacks into one pending refresh and discard stale fetch responses by request sequence so an earlier response cannot overwrite newer state. Cancel retry timers on cleanup. Preserve server replay history and make overflow explicit; only change backend ordering if the regression proves a defect. Event notifications are hints after committed state, never the durable audit itself.

```typescript
export function acceptEvent(last: number, value: unknown): { last: number; refresh: boolean } {
  if (typeof value !== 'object' || value === null) return { last, refresh: false };
  const event = value as Record<string, unknown>;
  if (event.version !== 1 || typeof event.event_id !== 'number' ||
      !Number.isSafeInteger(event.event_id) || event.event_id < 0 ||
      typeof event.event_type !== 'string') return { last, refresh: false };
  if (event.event_type === 'snapshot_refresh') {
    return { last: event.event_id, refresh: true };
  }
  return { last: Math.max(last, event.event_id), refresh: event.event_id > last };
}
```

- [ ] **Step 4: Verify the deliverable**

Run: `cd frontend && bun run test src/web/lib/admin-events.test.ts`
Expected after the change: replay duplicates produce one refresh; overflow and reconnect restore current views.

Add malformed frame, retained-history gap, session restart, reconnect cursor, cleanup-timer and out-of-order snapshot response tests. Run cargo test -p hiero --test daemon_ws_routes and frontend typecheck. No event credential payloads or new CSRF token.

- [ ] **Step 5: Review and commit this task**

Check the diff against the cited ADR/spec and the coverage row. Stage only this task's files; include a Cargo.lock change only if this task changes dependencies.

```bash
git add frontend/src/web/lib/admin-events.svelte.ts frontend/src/web/components/MemoryViews.svelte frontend/src/web/lib/admin-events.test.ts crates/hiero/src/daemon/events.rs crates/hiero/tests/daemon_ws_routes.rs
git commit -m "fix: resume admin events and avoid stale refreshes"
```

## Plan self-review

Coverage is mapped in the coordinator. Every task above has a regression, implementation sketch, explicit interfaces, and a focused verification command. Execute prerequisites first; snippets using newly introduced APIs intentionally fail to compile before those APIs land. Do not interpret a passing compile as the behavioral green step.

