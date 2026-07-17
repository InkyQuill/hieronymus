# Deep Reading And Coverage-Driven Dreaming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace alpha dreaming with evidence-tracked multi-pass runs, add book-scale batch reading memory ingestion, and normalize supported document formats before RAG ingestion.

**Architecture:** Keep direct source content in RAG and route agent conclusions through an atomic batch workspace API. Replace the existing mixed `DreamOutput` provider contract and old workflow names with seven independently configured pass contracts, staged output, deterministic coverage/deduplication, and one run-wide commit. Concepts remain soft; only rule crystals retain strict validation semantics.

**Tech Stack:** Python 3.12, SQLite, FastMCP, Click, pytest, `mammoth`, `pypdf`, HTML-to-Markdown library, existing Svelte web config/admin API.

## Global Constraints

- This is a breaking alpha migration: remove old Dream workflow names and compatibility behavior.
- Batch size accepts 500 items; every item is fully validated before one transaction writes any row.
- Text/Markdown import unchanged; HTML/DOCX/PDF normalize to managed Markdown; EPUB is rejected.
- Each Dream pass receives the same bounded complete-session selection and supplies source-memory evidence.
- Any required pass or coverage failure rolls back the entire Dream run.
- Concepts and variants are advisory; only rule crystals drive strict validation.

---

### Task 1: Add normalized RAG source ingestion

**Files:**
- Modify: `pyproject.toml`, `uv.lock`
- Create: `src/hieronymus/rag_conversion.py`
- Modify: `src/hieronymus/rag.py`, `src/hieronymus/rag_store.py`, `src/hieronymus/rag_payloads.py`, `src/hieronymus/mcp_server.py`
- Modify: `tests/test_rag_parsing.py`, `tests/test_rag_store.py`, `tests/test_mcp_rag.py`

**Interfaces:**
- Produces: `normalize_rag_source(path: Path, managed_root: Path) -> NormalizedRagSource`.
- Preserves: `hieronymus_rag_import` signature while extending its payload with `normalized_path` and `normalized_format`.

- [ ] **Step 1: Write failing normalization tests**

```python
def test_text_and_markdown_sources_are_not_transformed(tmp_path: Path) -> None:
    source = tmp_path / "chapter.md"
    source.write_text("# Chapter\n\nText", encoding="utf-8")
    normalized = normalize_rag_source(source, tmp_path / "managed")
    assert normalized.path == source
    assert normalized.format == "markdown"

def test_epub_is_rejected_with_actionable_error(tmp_path: Path) -> None:
    source = tmp_path / "book.epub"
    source.write_bytes(b"epub")
    with pytest.raises(ValueError, match="split EPUB"):
        normalize_rag_source(source, tmp_path / "managed")

def test_pdf_without_text_is_rejected(tmp_path: Path) -> None:
    source = tmp_path / "scan.pdf"
    source.write_bytes(make_empty_pdf())
    with pytest.raises(ValueError, match="no extractable text"):
        normalize_rag_source(source, tmp_path / "managed")
```

- [ ] **Step 2: Verify red**

Run: `uv run pytest tests/test_rag_parsing.py tests/test_rag_store.py tests/test_mcp_rag.py -v`

Expected: import/collection fails because `rag_conversion` is absent.

- [ ] **Step 3: Implement conversion and managed-artifact provenance**

```python
@dataclass(frozen=True)
class NormalizedRagSource:
    path: Path
    format: str
    original_path: Path

def normalize_rag_source(path: Path, managed_root: Path) -> NormalizedRagSource:
    suffix = path.suffix.casefold()
    if suffix in {".txt", ".md", ".markdown"}:
        return NormalizedRagSource(path, "markdown" if suffix != ".txt" else "text", path)
    if suffix == ".epub":
        raise ValueError("EPUB is unsupported; split EPUB into chapters before RAG import")
    # html/docx/pdf write deterministic managed_root/<sha256>.md, never mutate path
```

Use library conversion for HTML/DOCX/PDF, deterministic Markdown normalization, and content-hash-based managed artifact names. Persist original and normalized source references in the RAG import row; expose both in MCP payload.

- [ ] **Step 4: Verify green and commit**

Run: `uv run pytest tests/test_rag_parsing.py tests/test_rag_store.py tests/test_mcp_rag.py -v`

Expected: all direct, converted, error, and provenance tests pass.

Commit: `git add pyproject.toml uv.lock src/hieronymus/rag_conversion.py src/hieronymus/rag.py src/hieronymus/rag_store.py src/hieronymus/rag_payloads.py src/hieronymus/mcp_server.py tests/test_rag_parsing.py tests/test_rag_store.py tests/test_mcp_rag.py && git commit -m "feat: normalize supported RAG sources to markdown"`

### Task 2: Add atomic batch short-term-memory ingestion

**Files:**
- Modify: `src/hieronymus/workspace.py`, `src/hieronymus/mcp_server.py`, `src/hieronymus/mcp_operations.py`
- Modify: `tests/test_workspace.py`, `tests/test_mcp_agent_ingestion.py`, `tests/test_daemon_mcp_client.py`

**Interfaces:**
- Produces: `WorkspaceStore.add_short_term_memories(session_id, items) -> list[int]` and `hieronymus_short_term_add_batch(session_id, items) -> {memory_ids, count}`.
- Uses: exact existing per-item `add_short_term_memory` validation fields.

- [ ] **Step 1: Write failing batch tests**

```python
def test_batch_writes_500_items_in_order(config, active_session) -> None:
    items = [short_memory_input(text=f"Conclusion {index}.") for index in range(500)]
    ids = WorkspaceStore(config).add_short_term_memories(active_session.id, items)
    assert ids == list(range(1, 501))

def test_invalid_batch_item_rolls_back_every_item(config, active_session) -> None:
    items = [short_memory_input(text="Valid."), short_memory_input(text="")]
    with pytest.raises(ValueError, match="text"):
        WorkspaceStore(config).add_short_term_memories(active_session.id, items)
    assert WorkspaceStore(config).list_short_term_memories(active_session.id) == []
```

- [ ] **Step 2: Verify red**

Run: `uv run pytest tests/test_workspace.py tests/test_mcp_agent_ingestion.py -v`

Expected: missing batch method/tool failures.

- [ ] **Step 3: Implement transaction and MCP tool**

```python
MAX_SHORT_TERM_MEMORY_BATCH = 500

def add_short_term_memories(self, session_id: int, items: Sequence[ShortTermMemoryInput]) -> list[int]:
    if not 1 <= len(items) <= MAX_SHORT_TERM_MEMORY_BATCH:
        raise ValueError("short-term memory batch must contain 1-500 items")
    validated = [self._validate_short_term_input(item) for item in items]
    with connect(self.config.database_path) as conn:
        self._require_active_session(conn, session_id)
        return self._insert_validated_short_term_memories(conn, session_id, validated)
```

Validate all items before SQL insertion; use one transaction; return IDs in input order. Register the new operation in daemon MCP routing and make server errors retain the existing ValueError contract.

- [ ] **Step 4: Verify green and commit**

Run: `uv run pytest tests/test_workspace.py tests/test_mcp_agent_ingestion.py tests/test_daemon_mcp_client.py -v`

Expected: batch atomicity, order, 500-item acceptance, daemon delegation, and original single-add behavior pass.

Commit: `git add src/hieronymus/workspace.py src/hieronymus/mcp_server.py src/hieronymus/mcp_operations.py tests/test_workspace.py tests/test_mcp_agent_ingestion.py tests/test_daemon_mcp_client.py && git commit -m "feat: add batch short-term memory ingestion"`

### Task 3: Make concepts soft and remove divergent-variant rejection

**Files:**
- Modify: `src/hieronymus/admin.py`, `src/hieronymus/termbase.py`, `src/hieronymus/dreaming.py`
- Modify: `tests/test_admin_actions.py`, `tests/test_termbase_validate.py`, `tests/test_dreaming.py`

**Interfaces:**
- Consumes: approved concept proposal variants.
- Produces: concept/facet/soft alias updates; strict rule crystals only for explicit rule evidence.

- [ ] **Step 1: Write failing approval tests**

```python
def test_approval_accepts_divergent_advisory_variants(config, proposal_id) -> None:
    concept_id = AdminStore(config).approve_proposal(proposal_id)
    assert ConceptStore(config).get(concept_id).canonical_name == "Sense"
    assert "Feeling" in ConceptStore(config).list_facets(concept_id)[0].alternate_renderings

def test_advisory_variant_does_not_relax_rule_validation(config, approved_rule) -> None:
    findings = Termbase(config).validate_text("Wrong rendering", language="en")
    assert findings[0].kind == "required_form"
```

- [ ] **Step 2: Verify red**

Run: `uv run pytest tests/test_admin_actions.py tests/test_termbase_validate.py tests/test_dreaming.py -v`

Expected: divergent-variant rejection persists.

- [ ] **Step 3: Remove strict alias coupling**

Route proposal approval through concepts/facets and advisory aliases. Do not call rule-crystal text construction for ordinary concept proposals. Preserve `RuleCrystal` creation only for explicit `rule_intent`/user-rule evidence and retain canonical/forbidden validation there.

- [ ] **Step 4: Verify green and commit**

Run: `uv run pytest tests/test_admin_actions.py tests/test_termbase_validate.py tests/test_dreaming.py -v`

Expected: soft approval succeeds and strict validation remains rule-only.

Commit: `git add src/hieronymus/admin.py src/hieronymus/termbase.py src/hieronymus/dreaming.py tests/test_admin_actions.py tests/test_termbase_validate.py tests/test_dreaming.py && git commit -m "refactor: keep concept variants advisory"`

### Task 4: Replace Dream workflow/config/provider contracts

**Files:**
- Modify: `src/hieronymus/dream_config.py`, `src/hieronymus/dream_workflows.py`, `src/hieronymus/dream_providers.py`, `src/hieronymus/dreaming.py`, `src/hieronymus/dream_audit.py`, `src/hieronymus/dream_autostart.py`
- Modify: `tests/test_dream_config.py`, `tests/test_dream_workflows.py`, `tests/test_dream_providers.py`, `tests/test_dreaming.py`, `tests/test_dream_audit.py`, `tests/test_dream_autostart.py`

**Interfaces:**
- Replaces: old workflow names and mixed `DreamOutput(crystals, concept_proposals)`.
- Produces: seven typed pass outputs with source-memory evidence and run-wide staged commit.

- [ ] **Step 1: Write failing configuration and provider-contract tests**

```python
def test_default_dream_config_has_exactly_seven_new_passes() -> None:
    assert tuple(default_dream_config().workflows) == (
        "concepts", "terminology_candidates", "rule_crystals", "knowledge_crystals",
        "relations", "reinforcement", "coverage_audit",
    )

def test_pass_record_without_selected_source_memory_is_rejected(config) -> None:
    with pytest.raises(ValueError, match="source_memory_ids"):
        DreamService(config, provider_with_unknown_source()).run_cycle()
```

- [ ] **Step 2: Verify red**

Run: `uv run pytest tests/test_dream_config.py tests/test_dream_workflows.py tests/test_dream_providers.py tests/test_dreaming.py -v`

Expected: old-phase/default-output tests fail.

- [ ] **Step 3: Implement the breaking contract**

Define `DreamPassOutput` variants for concepts, terminology candidates, rules, knowledge crystals, relations, reinforcement, and coverage classification. Delete old aliases/workflow mappings. Give every pass independent provider/model/prompt configuration and `max_records_per_pass`; add run limits `max_short_term_memories_per_run`, `max_long_term_records_affected_per_run`, and `max_relation_records_per_pass`. Replace old config load behavior with the new schema; old names are validation errors.

- [ ] **Step 4: Implement staging, selection, and rollback**

Select completed session groups oldest-first without splitting a group, up to 500 memories. Invoke all seven passes with the identical selected IDs. Parse/validate/stage output, reject output over limits or unsupported evidence, normalize exact duplicates, and commit once. On any pass error, rollback all writes and return sessions to completed. Reinforce each valid long-term reference once. Compute coverage from accepted staged evidence and fail/rollback with `coverage_incomplete` for any uncovered selected ID.

- [ ] **Step 5: Verify green and commit**

Run: `uv run pytest tests/test_dream_config.py tests/test_dream_workflows.py tests/test_dream_providers.py tests/test_dreaming.py tests/test_dream_audit.py tests/test_dream_autostart.py -v`

Expected: seven-pass contract, same-set selection, bounds, duplicate handling, reinforcement, complete rollback, and coverage failure tests pass.

Commit: `git add src/hieronymus/dream_config.py src/hieronymus/dream_workflows.py src/hieronymus/dream_providers.py src/hieronymus/dreaming.py src/hieronymus/dream_audit.py src/hieronymus/dream_autostart.py tests/test_dream_config.py tests/test_dream_workflows.py tests/test_dream_providers.py tests/test_dreaming.py tests/test_dream_audit.py tests/test_dream_autostart.py && git commit -m "refactor: replace dreaming with evidence passes"`

### Task 5: Wire Read, web configuration, admin audit, and documentation

**Files:**
- Modify: `src/hieronymus/agent_assets.py`, `src/hieronymus/service_http.py`, `src/hieronymus/tui_bridge/config_api.py`, `src/hieronymus/tui_bridge/admin_api.py`, `frontend/src/**`
- Modify: `docs/agent-workflows.md`, `docs/memory-dreaming.md`, `docs/usage.md`
- Modify: `tests/test_agent_assets.py`, `tests/test_tui_bridge_config.py`, `tests/test_tui_bridge_admin.py`, `tests/test_service_http.py`

**Interfaces:**
- Consumes: Tasks 1–4 MCP/config/audit contracts.
- Produces: generated Read batch workflow and inspectable seven-pass web surfaces.

- [ ] **Step 1: Write failing UI/skill contract tests**

```python
def test_read_skill_requires_rag_import_then_batch_conclusions() -> None:
    read = asset_map()["skills/hieronymus-read/SKILL.md"]
    assert "hieronymus_rag_import" in read
    assert "hieronymus_short_term_add_batch" in read
    assert "1–6 sentences" in read

def test_admin_run_payload_has_per_pass_coverage_counts(config) -> None:
    payload = AdminBridge(config).snapshot({"view": "Dream runs"})
    assert "covered_count" in payload["items"][0]["passes"][0]
```

- [ ] **Step 2: Verify red**

Run: `uv run pytest tests/test_agent_assets.py tests/test_tui_bridge_config.py tests/test_tui_bridge_admin.py tests/test_service_http.py -v`

Expected: batch/pass coverage payload assertions fail.

- [ ] **Step 3: Implement surfaces and documentation**

Update generated Read instructions to call RAG import then bounded batch adds. Replace old Dream configuration fields in API/frontend with seven pass profiles and four limits. Expose per-pass counts/evidence/rollback/coverage in admin. Document breaking alpha behavior, source format handling, soft concepts, and rule-only strict validation.

- [ ] **Step 4: Verify full feature and commit**

Run: `uv run pytest tests/test_agent_assets.py tests/test_tui_bridge_config.py tests/test_tui_bridge_admin.py tests/test_service_http.py -v && bun run --cwd frontend typecheck && bun run --cwd frontend build`

Expected: skill/API/UI tests and frontend checks pass.

Commit: `git add src/hieronymus/agent_assets.py src/hieronymus/service_http.py src/hieronymus/tui_bridge/config_api.py src/hieronymus/tui_bridge/admin_api.py frontend docs/agent-workflows.md docs/memory-dreaming.md docs/usage.md tests/test_agent_assets.py tests/test_tui_bridge_config.py tests/test_tui_bridge_admin.py tests/test_service_http.py && git commit -m "feat: expose deep reading and dream coverage workflows"`

### Task 6: Book-scale integration and final verification

**Files:**
- Create: `tests/test_deep_read_dream_integration.py`
- Modify: relevant test fixtures only if required by Tasks 1–5

**Interfaces:**
- Consumes: complete batch/RAG/Dream workflow.
- Produces: a verified 500-memory book-scale regression.

- [ ] **Step 1: Write the failing end-to-end regression**

```python
def test_book_scale_reading_runs_all_passes_without_silent_loss(config, active_session) -> None:
    ids = DaemonMcpClient(config).invoke("short_term_add_batch", {"session_id": active_session.id, "items": book_items(500)})["memory_ids"]
    run = DreamService(config, exhaustive_fake_provider()).run_cycle()
    assert run.status == "completed"
    assert audit_covered_memory_ids(config, run.id) == set(ids)
```

- [ ] **Step 2: Verify red, then green**

Run: `uv run pytest tests/test_deep_read_dream_integration.py -v`

Expected before completion: failure at the missing pass/coverage contract; after implementation: PASS with all 500 IDs covered.

- [ ] **Step 3: Run repository verification, build, install, and push**

Run: `uv run pytest`, `uv run ruff check .`, `uv run ruff format --check .`, `bun run --cwd frontend typecheck`, `bun run --cwd frontend build`, `uv build`, `uv tool install --reinstall dist/hieronymus-*.whl`, `hiero restart --json`, and `git push origin main`.

Expected: all checks pass; installed daemon reports the built version and the branch pushes cleanly.
