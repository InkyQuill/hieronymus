# Provider-Backed Dreaming Audit Smoke Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lock the remaining Memory And Dreaming roadmap slice with provider-backed multi-batch smoke coverage that proves crystallization and system maintenance share one auditable dream run.

**Architecture:** Keep the current `DreamProvider` contract unchanged: providers only implement `crystallize()`. Maintenance is currently system-owned passive reinforcement and cycle decay, so this slice verifies a provider-backed crystallization run that continues into maintenance and records complete audit payloads for both phases. Production changes are limited to filling real audit gaps found by the new tests.

**Tech Stack:** Python 3.12, pytest, SQLite, existing `DreamService`, `DreamAuditStore`, `WorkspaceStore`, `CrystalStore`, and `FeedbackStore`.

---

## Current Code Map

- `src/hieronymus/dreaming.py`
  - `DreamProvider` has one method, `crystallize(context, memories)`.
  - `_run_cycle_unlocked()` creates one `dream_runs` row, one crystallization `dream_phase_runs` row per processed batch, and optionally one maintenance `dream_phase_runs` row when passive events or decay candidates exist.
  - `_audit_provider_request()`, `_audit_provider_response()`, `_audit_parse_warnings()`, and `_audit_phase_completed()` already write crystallization audit entries.
  - Maintenance currently uses `_audit_phase_completed(..., phase_name="maintenance")` with empty request/response summaries and populated mutation fields such as `reinforced_crystals`, `decayed_crystals`, `affected_memory_set`, and `skipped_candidates`.
  - `_summary_output_count()` already counts maintenance mutation IDs. Crystallization phase `output_count` is intentionally narrower in `_run_cycle_unlocked()`: created crystals plus concept proposals, not concepts and facets.
- `tests/test_dream_bounded_audit.py`
  - Contains reusable helpers for bounded provider-backed dreaming: `_context()`, `_completed_session()`, `_save_dreaming_config()`, `_add_crystal()`, `_phase_completed_payloads()`, and `_dream_audit_labels()`.
  - Already covers provider request/response/parse-warning payloads for crystallization and maintenance-only audit visibility.
  - Does not yet cover a single provider-backed run that has multiple crystallization batches and a maintenance phase.
- `tests/test_dreaming.py`
  - Covers batch draining, failure behavior, passive reinforcement, cycle decay, and activation bookkeeping.
  - Lower-level maintenance behavior is already tested here; this plan should not duplicate those tests.
- `docs/roadmap.md`
  - Memory And Dreaming remaining work is now specifically about provider-backed multi-phase smoke coverage and maintenance audit coverage.

## Implementation Boundary

This plan does not add provider-generated maintenance decisions. The product may add that later as a separate design decision. For this slice, "provider-backed multi-phase" means:

1. A non-deterministic test provider receives multiple crystallization batches.
2. The same dream run then performs maintenance because passive feedback and decay candidates exist.
3. Audit rows and phase rows make the complete run understandable from the admin/debug surfaces.

Completing this plan closes both current Memory And Dreaming roadmap bullets:

- Provider-backed dreaming smoke coverage exercises multi-phase provider payloads through crystallization and maintenance paths.
- Dream audit coverage includes maintenance decisions and multi-phase provider runs.

---

### Task 1: Add Provider-Backed Multi-Phase Smoke Test

**Files:**
- Modify: `tests/test_dream_bounded_audit.py`

- [ ] **Step 1: Add a helper for phase rows**

In `tests/test_dream_bounded_audit.py`, add this helper after `_phase_input_counts()`:

```python
def _phase_rows(config: HieronymusConfig) -> list[dict[str, object]]:
    with connect(config.database_path) as conn:
        rows = conn.execute(
            """
            select phase, status, input_count, output_count
            from dream_phase_runs
            order by id
            """
        ).fetchall()
    return [dict(row) for row in rows]
```

- [ ] **Step 2: Add the failing multi-phase smoke test**

In `tests/test_dream_bounded_audit.py`, add this test after `test_affected_set_and_phase_audit_payloads_are_bounded_and_complete()`:

```python
def test_provider_backed_multi_batch_run_audits_crystallization_and_maintenance(
    config: HieronymusConfig,
) -> None:
    _save_dreaming_config(config, min_pending=1, max_pending=10, per_cycle=2, max_changed=4)
    context = _context(config)
    memory_ids = _completed_session(config, context, memories=3)
    reinforced_id = _add_crystal(
        config,
        context,
        text="Provider-backed maintenance reinforcement.",
        strength=0.5,
        confidence=0.5,
    )
    decayed_id = _add_crystal(
        config,
        context,
        text="Provider-backed maintenance decay.",
        strength=0.5,
        confidence=0.5,
    )
    FeedbackStore(config).record(
        reinforced_id,
        event_type="used_in_translation",
        source_role="system",
        evidence="used during provider-backed smoke",
    )
    provider = CapturingDictProvider()

    run = DreamService(config, provider).run_all(owner="admin")

    assert run.status == "completed"
    assert run.provider == "capturing"
    assert run.input_count == 3
    assert provider.calls == [memory_ids[:2], memory_ids[2:]]
    assert _phase_rows(config) == [
        {"phase": "crystallization", "status": "completed", "input_count": 2, "output_count": 1},
        {"phase": "crystallization", "status": "completed", "input_count": 1, "output_count": 1},
        {"phase": "maintenance", "status": "completed", "input_count": 0, "output_count": 2},
    ]

    entries = DreamAuditStore(config).list_for_run(run.id)
    assert [entry.event_type for entry in entries] == [
        "provider_request",
        "parse_warnings",
        "provider_response",
        "phase_completed",
        "provider_request",
        "parse_warnings",
        "provider_response",
        "phase_completed",
        "phase_completed",
    ]
    phase_payloads = [entry.payload for entry in entries if entry.event_type == "phase_completed"]
    assert [payload["phase_name"] for payload in phase_payloads] == [
        "crystallization",
        "crystallization",
        "maintenance",
    ]
    assert phase_payloads[0]["selected_short_term_memory_ids"] == memory_ids[:2]
    assert phase_payloads[1]["selected_short_term_memory_ids"] == memory_ids[2:]
    maintenance_payload = phase_payloads[2]
    assert maintenance_payload["selected_short_term_memory_ids"] == []
    assert maintenance_payload["provider_profile"] == "capturing"
    assert maintenance_payload["model"] == "capturing"
    assert maintenance_payload["prompt_version"] == "maintenance:v1"
    assert maintenance_payload["reinforced_crystals"] == [reinforced_id]
    assert maintenance_payload["decayed_crystals"] == [decayed_id]
    assert set(maintenance_payload["affected_memory_set"]["changed_crystal_ids"]) == {
        reinforced_id,
        decayed_id,
    }
    assert maintenance_payload["request_summary"] == {
        "memory_count": 0,
        "session_ids": [],
        "context_count": 0,
        "batch_cap": 2,
    }
    assert maintenance_payload["response_summary"] == {
        "crystal_count": 0,
        "concept_count": 0,
        "facet_count": 0,
        "concept_proposal_count": 0,
        "supersede_action_count": 0,
        "parse_warning_count": 0,
    }
```

- [ ] **Step 3: Run the new smoke test**

```bash
uv run pytest tests/test_dream_bounded_audit.py::test_provider_backed_multi_batch_run_audits_crystallization_and_maintenance -q
```

Expected: FAIL only if the current phase rows or maintenance payload are missing a required value. If it passes, do not change production code in this task.

- [ ] **Step 4: Apply the only expected production fix if needed**

The current `src/hieronymus/dreaming.py` implementation should already match the test. If Step 3 fails because maintenance `output_count` does not count reinforced and decayed crystals, restore `_summary_output_count()` to this implementation:

```python
def _summary_output_count(summary: _DreamApplySummary) -> int:
    return sum(
        len(values)
        for values in (
            summary.created_crystal_ids,
            summary.created_concept_ids,
            summary.created_facet_ids,
            summary.created_links,
            summary.superseded_crystal_ids,
            summary.reinforced_crystal_ids,
            summary.decayed_crystal_ids,
        )
    )
```

If Step 3 fails for a different reason, first check whether the test expected value is inconsistent with the current code map above. Fix the test expectation when the production behavior is already intentional. Do not add `provider_request` or `provider_response` events for maintenance in this task; maintenance has no provider call in the current architecture.

- [ ] **Step 5: Re-run the smoke test**

```bash
uv run pytest tests/test_dream_bounded_audit.py::test_provider_backed_multi_batch_run_audits_crystallization_and_maintenance -q
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/hieronymus/dreaming.py tests/test_dream_bounded_audit.py
git commit -m "test: cover provider-backed multi-phase dreaming audit"
```

---

### Task 2: Lock Admin Audit Visibility For Multi-Phase Runs

**Files:**
- Modify: `tests/test_dream_bounded_audit.py`

- [ ] **Step 1: Add a focused admin ordering test**

In `tests/test_dream_bounded_audit.py`, add this test after `test_admin_bridge_audit_lookup_returns_phase_entries_and_parse_warning_records()`:

```python
def test_admin_bridge_audit_lookup_includes_provider_backed_maintenance_phase(
    config: HieronymusConfig,
) -> None:
    _save_dreaming_config(config, min_pending=1, max_pending=10, per_cycle=2, max_changed=3)
    context = _context(config)
    _completed_session(config, context, memories=2)
    reinforced_id = _add_crystal(
        config,
        context,
        text="Admin visible provider-backed maintenance.",
        strength=0.5,
        confidence=0.5,
    )
    FeedbackStore(config).record(
        reinforced_id,
        event_type="used_in_translation",
        source_role="system",
        evidence="visible in admin audit smoke",
    )

    run = DreamService(config, CapturingDictProvider()).run_all(owner="admin")

    payload = AdminBridge(config).snapshot({"view": "Dream Audits"})
    labels = [row["label"] for row in payload["snapshot"]["rows"]]
    assert labels[:5] == [
        "phase_completed: completed maintenance phase",
        "phase_completed: completed crystallization phase",
        "provider_response: received crystallization response",
        "parse_warnings: dream response parsed with recoverable warnings",
        "provider_request: sent crystallization request",
    ]
    assert payload["snapshot"]["detail"]["title"] == labels[0]
    assert '"phase_name": "maintenance"' in payload["snapshot"]["detail"]["body"]
    assert str(reinforced_id) in payload["snapshot"]["detail"]["body"]

    entries = DreamAuditStore(config).list_for_run(run.id)
    assert entries[-1].event_type == "phase_completed"
    assert entries[-1].payload["phase_name"] == "maintenance"
```

- [ ] **Step 2: Run the admin audit test**

```bash
uv run pytest tests/test_dream_bounded_audit.py::test_admin_bridge_audit_lookup_includes_provider_backed_maintenance_phase -q
```

Expected: PASS. If it fails because the admin snapshot does not include the maintenance entry, inspect `src/hieronymus/tui_bridge/admin_api.py` and fix the existing Dream Audits query/filter so it returns all audit entries for a run, including maintenance `phase_completed` rows.

- [ ] **Step 3: Run bounded audit tests**

```bash
uv run pytest tests/test_dream_bounded_audit.py -q
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/hieronymus/tui_bridge/admin_api.py tests/test_dream_bounded_audit.py
git commit -m "test: cover multi-phase dream audit admin visibility"
```

---

### Task 3: Update Roadmap And Memory Dreaming Docs

**Files:**
- Modify: `docs/roadmap.md`
- Modify: `docs/memory-dreaming.md`

- [ ] **Step 1: Update the roadmap baseline**

In `docs/roadmap.md`, update the Memory And Dreaming section by replacing this completed baseline bullet:

```markdown
- Provider-backed crystallization audit coverage verifies provider request and
  response summaries, parse warnings, selected memory IDs, and affected memory
  set payloads.
```

with:

```markdown
- Provider-backed audit coverage verifies crystallization request and response
  summaries, parse warnings, selected memory IDs, affected memory set payloads,
  multi-batch phase runs, and maintenance phase decisions in the same dream run.
```

- [ ] **Step 2: Remove completed roadmap remaining bullets**

In `docs/roadmap.md`, remove these Memory And Dreaming remaining bullets after Tasks 1 and 2 pass:

```markdown
- Add provider-backed dreaming smoke coverage that exercises multi-phase
  provider payloads through crystallization and maintenance paths.
- Extend dream audit coverage to maintenance decisions and multi-phase provider
  runs.
```

After removing those bullets, replace the Memory And Dreaming `Remaining work:` block with:

```markdown
Remaining work:

- No active roadmap items in this section.
```

- [ ] **Step 3: Update detailed memory docs**

In `docs/memory-dreaming.md`, find the paragraph that starts with:

```markdown
Every dream cycle writes an audit record.
```

Add this sentence after that paragraph:

```markdown
Provider-backed smoke coverage exercises multiple crystallization batches and
the following maintenance phase in one dream run, so audit inspection can trace
provider input, provider output, parse warnings, and maintenance mutations
together.
```

- [ ] **Step 4: Commit docs**

```bash
git add docs/roadmap.md docs/memory-dreaming.md docs/usage.md
git commit -m "docs: update memory dreaming audit roadmap"
```

---

### Task 4: Final Verification And PR

**Files:**
- Verify only

- [ ] **Step 1: Run targeted Python tests**

```bash
uv run pytest tests/test_dream_bounded_audit.py tests/test_dreaming.py tests/test_dream_audit.py -q
```

Expected: PASS.

- [ ] **Step 2: Run full backend verification**

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected: PASS.

- [ ] **Step 3: Check the diff**

```bash
git diff --check
git status --short
```

Expected: `git diff --check` prints nothing. `git status --short` contains only intentional files before the final commit and is clean after commits.

- [ ] **Step 4: Push and open PR**

```bash
git push -u origin plan/provider-backed-dreaming-audit-smoke
gh pr create --fill
```

Expected: GitHub opens a PR from `plan/provider-backed-dreaming-audit-smoke` into `main`.

---

## Self-Review

Spec coverage:

- Roadmap says provider-backed multi-phase smoke coverage remains: Task 1 creates a provider-backed run with two crystallization batches plus maintenance in one dream run.
- Roadmap says maintenance audit coverage remains: Task 1 asserts the maintenance payload; Task 2 asserts admin-visible audit lookup.
- The plan avoids locking in future implementation: it does not add a provider maintenance method and explicitly documents the current provider boundary.
- Docs are updated after tests prove the work is real: Task 3 moves both current Memory And Dreaming remaining bullets into the completed baseline and leaves no active roadmap items in that section.

Completion-marker scan:

- No deferred-work markers are present.
- Every task lists exact files, test names, commands, and expected outcomes.

Type consistency:

- Test snippets use existing helpers from `tests/test_dream_bounded_audit.py`.
- Production references use current names in `src/hieronymus/dreaming.py`: `_summary_output_count()`, `_audit_phase_completed()`, and `DreamProvider.crystallize()`.
- Audit payload keys match current `_audit_phase_completed()` fields.
