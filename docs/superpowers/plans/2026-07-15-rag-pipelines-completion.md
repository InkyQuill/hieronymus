# RAG Pipelines Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the RAG MVP plan with robust import validation, format-aware reindexing, explicit module/test boundaries, isolated release tests, and complete verification evidence.

**Architecture:** Keep `hieronymus.rag` as a compatibility facade. Move parsed-file contracts and all parsing/chunking behavior to `rag_parsing.py`; move SQLite persistence, retrieval, hydration, and ranking to `rag_store.py`. Preserve all existing CLI, MCP, and recall interfaces.

**Tech Stack:** Python 3.12, SQLite FTS5, PyYAML, pytest, Ruff, `uv`.

## Global Constraints

- Preserve imports of `RagStore`, `load_rag_file`, `ParsedRagFile`, and `ParsedRagChunk` from `hieronymus.rag`.
- Do not change ranking weights, FTS queries, recall budgets, rule-crystal protection, transaction boundaries, or activation behavior.
- Do not add embeddings, directory import, admin screens, or other RAG backlog features.
- Parsing failures must occur before database mutation and preserve an existing indexed source.
- Full-suite tests must not invoke or remove the developer's real installed Hieronymus tool.
- Use red-green TDD for every behavior change.

---

## File Structure

- Create `src/hieronymus/rag_parsing.py`: parsed dataclasses, supported-type resolution, checksum calculation, parsing, chunking, and glossary validation.
- Create `src/hieronymus/rag_store.py`: `RagStore`, SQLite helpers, FTS search, scoring, and rank reasons.
- Replace `src/hieronymus/rag.py` with a compatibility facade that re-exports the public API.
- Create `tests/test_rag_schema.py`: schema, foreign-key, and FTS-trigger invariants.
- Create `tests/test_rag_parsing.py`: parsed dataclasses, source validation, parsing, and chunking.
- Keep `tests/test_rag_store.py`: import, replacement, metadata, retrieval, and ranking behavior only.
- Modify `tests/test_release_scripts.py`: deterministic fake `uv` for script execution.
- Modify `tests/test_recall_enriched_memory.py`: explicit RAG rank/score assertions.
- Modify `docs/superpowers/plans/2026-07-04-rag-pipelines.md`: close Task 7 after verification.

## Task 1: Validate Delimited Glossary Headers

**Files:**
- Modify: `tests/test_rag_store.py`
- Modify: `src/hieronymus/rag.py`

**Interfaces:**
- Consumes: `load_rag_file(path: Path, *, source_type: str) -> ParsedRagFile`
- Produces: the same API with deterministic blank/duplicate header rejection.

- [x] **Step 1: Add failing parser tests**

Add next to the existing CSV parser tests:

```python
@pytest.mark.parametrize(
    ("header", "message"),
    [
        ("source,,target", "non-empty headers"),
        ("source, source ,target", "duplicate headers"),
    ],
)
def test_csv_file_rejects_invalid_headers(
    tmp_path: Path,
    header: str,
    message: str,
) -> None:
    path = tmp_path / "glossary.csv"
    path.write_text(f"{header}\nSense,alias,Сенс\n", encoding="utf-8")

    with pytest.raises(ValueError, match=message):
        load_rag_file(path, source_type="auto")
```

- [x] **Step 2: Run the tests and verify RED**

Run:

```bash
uv run pytest tests/test_rag_store.py::test_csv_file_rejects_invalid_headers -q
```

Expected: both cases fail because `csv.DictReader` currently accepts the headers.

- [x] **Step 3: Normalize and validate the header row before reading data rows**

At the start of `_parse_delimited_glossary` after constructing `DictReader`, add:

```python
        headers = [header.strip() for header in reader.fieldnames or ()]
        if not headers or any(not header for header in headers):
            raise ValueError("Delimited glossary requires non-empty headers")
        if len(headers) != len(set(headers)):
            raise ValueError("Delimited glossary contains duplicate headers")
        reader.fieldnames = headers
```

Keep the current extra-field detection and metadata construction unchanged.

- [x] **Step 4: Run parser tests and verify GREEN**

Run:

```bash
uv run pytest tests/test_rag_store.py -q
```

Expected: all RAG store/parser tests pass.

- [x] **Step 5: Commit the validation fix**

```bash
git add src/hieronymus/rag.py tests/test_rag_store.py
git commit -m "fix: validate rag glossary headers"
```

## Task 2: Reindex When Parsed Format Changes

**Files:**
- Modify: `tests/test_rag_store.py`
- Modify: `src/hieronymus/rag.py`

**Interfaces:**
- Consumes: `RagStore.import_file(..., source_ref: str, source_type: str = "auto")`.
- Produces: checksum fast path keyed by checksum plus parsed source/content types.

- [x] **Step 1: Add a failing format-change regression test**

```python
def test_same_checksum_with_changed_format_reindexes_source(
    config: HieronymusConfig,
    tmp_path: Path,
) -> None:
    series_slug = _series(config)
    text_path = tmp_path / "source.txt"
    markdown_path = tmp_path / "source.md"
    content = "# Sense\n\nSense menu note."
    text_path.write_text(content, encoding="utf-8")
    markdown_path.write_text(content, encoding="utf-8")
    store = RagStore(config)

    first = store.import_file(series_slug, text_path, source_ref="source", source_type="auto")
    second = store.import_file(
        series_slug,
        markdown_path,
        source_ref="source",
        source_type="auto",
    )
    hit = store.search(series_slug, "Sense menu", limit=1)[0]

    assert first.source.content_type == "txt"
    assert second.skipped is False
    assert second.source.source_type == "markdown"
    assert second.source.content_type == "md"
    assert hit.chunk.chunk_kind == "markdown_section"
    assert hit.chunk.location == "Sense paragraph 1"
```

- [x] **Step 2: Run the regression test and verify RED**

Run:

```bash
uv run pytest tests/test_rag_store.py::test_same_checksum_with_changed_format_reindexes_source -q
```

Expected: FAIL because the second import returns the stored TXT source with `skipped=True`.

- [x] **Step 3: Make the fast path format-aware**

Replace its condition with:

```python
            if (
                existing is not None
                and existing["checksum"] == parsed.checksum
                and existing["source_type"] == parsed.source_type
                and existing["content_type"] == parsed.content_type
            ):
```

Keep tag refresh and normal replacement behavior unchanged.

- [x] **Step 4: Run store tests and verify GREEN**

Run:

```bash
uv run pytest tests/test_rag_store.py -q
```

Expected: all tests pass, including unchanged-checksum tag refresh.

- [x] **Step 5: Commit the format-aware import fix**

```bash
git add src/hieronymus/rag.py tests/test_rag_store.py
git commit -m "fix: refresh rag source when format changes"
```

## Task 3: Isolate Uninstall-Script Tests

**Files:**
- Modify: `tests/test_release_scripts.py`

**Interfaces:**
- Consumes: `script_env(tmp_path, *, home=None) -> dict[str, str]`.
- Produces: a temporary PATH that always contains a harmless fake `uv`, unless a test already provided its own fake executable.

- [x] **Step 1: Use the existing successful uninstall test as the RED reproduction**

Run with a short external timeout:

```bash
timeout 10s uv run pytest tests/test_release_scripts.py::test_uninstall_keep_data_removes_safe_app_and_keeps_safe_data -q
```

Expected before the fix in the current developer environment: exit 124 because the test reaches the real `uv tool uninstall hieronymus`.

- [x] **Step 2: Add a default fake `uv` to `script_env`**

After `fake_bin.mkdir(exist_ok=True)`, add:

```python
    fake_uv = fake_bin / "uv"
    if not fake_uv.exists():
        write_executable(
            fake_uv,
            """
            #!/bin/sh
            exit 0
            """,
        )
```

The existence guard preserves specialized fake `uv` scripts installed by other tests before they call `script_env`.

- [x] **Step 3: Verify the isolated test is GREEN**

Run:

```bash
timeout 10s uv run pytest tests/test_release_scripts.py::test_uninstall_keep_data_removes_safe_app_and_keeps_safe_data -q
```

Expected: PASS without touching the real installed tool.

- [x] **Step 4: Run the entire release-script module**

Run:

```bash
uv run pytest tests/test_release_scripts.py -q
```

Expected: all release-script tests pass without network calls.

- [x] **Step 5: Commit test isolation**

```bash
git add tests/test_release_scripts.py
git commit -m "test: isolate uninstall script from installed tool"
```

## Task 4: Split Parsing From Storage Behind A Compatibility Facade

**Files:**
- Create: `src/hieronymus/rag_parsing.py`
- Create: `src/hieronymus/rag_store.py`
- Modify: `src/hieronymus/rag.py`

**Interfaces:**
- Produces in `rag_parsing.py`: `RagLoadSourceType`, `MAX_RAG_CHUNK_CHARS`, `ParsedRagChunk`, `ParsedRagFile`, and `load_rag_file`.
- Produces in `rag_store.py`: `RagStore`.
- Preserves in `rag.py`: re-exports of all six public names.

- [x] **Step 1: Record the pre-refactor focused baseline**

Run:

```bash
uv run pytest tests/test_rag_store.py tests/test_rag_cli.py tests/test_mcp_rag.py tests/test_recall_rag.py -q
```

Expected: PASS.

- [x] **Step 2: Extract parsing and chunking without behavior changes**

Move these definitions and their direct dependencies to `rag_parsing.py`:

```text
RagLoadSourceType
MAX_RAG_CHUNK_CHARS
ParsedRagChunk
ParsedRagFile
load_rag_file
_resolve_source_type
_parse_text
_parse_markdown
_parse_delimited_glossary
_parse_json_glossary
_parse_yaml_glossary
_parse_structured_glossary
_glossary_entries
_metadata_from_value
_glossary_chunk
_glossary_text
_paragraphs
_located_chunk_texts
_split_chunk_text
_sentence_segments
_word_chunks
_hard_chunks
```

`rag_parsing.py` imports only parsing dependencies (`csv`, `hashlib`, `json`,
`re`, dataclasses, `Path`, typing, `yaml`, and RAG model literals). It contains no
database, config, crystal-search, or store imports.

- [x] **Step 3: Extract store and ranking without behavior changes**

Move `RagStore` and all remaining private storage/ranking helpers to
`rag_store.py`. Import the parser API explicitly:

```python
from hieronymus.rag_parsing import RagLoadSourceType, load_rag_file
```

`rag_store.py` contains database/config/model imports but no CSV, hashing,
regular-expression, or YAML imports.

- [x] **Step 4: Replace `rag.py` with the compatibility facade**

```python
from hieronymus.rag_parsing import (
    MAX_RAG_CHUNK_CHARS,
    ParsedRagChunk,
    ParsedRagFile,
    RagLoadSourceType,
    load_rag_file,
)
from hieronymus.rag_store import RagStore

__all__ = [
    "MAX_RAG_CHUNK_CHARS",
    "ParsedRagChunk",
    "ParsedRagFile",
    "RagLoadSourceType",
    "RagStore",
    "load_rag_file",
]
```

- [x] **Step 5: Run focused tests after extraction**

Run:

```bash
uv run pytest tests/test_rag_store.py tests/test_rag_cli.py tests/test_mcp_rag.py tests/test_recall_rag.py -q
uv run ruff check src/hieronymus/rag.py src/hieronymus/rag_parsing.py src/hieronymus/rag_store.py
```

Expected: all tests and lint checks pass with unchanged caller imports.

- [x] **Step 6: Commit the production module split**

```bash
git add src/hieronymus/rag.py src/hieronymus/rag_parsing.py src/hieronymus/rag_store.py
git commit -m "refactor: split rag parsing and storage"
```

## Task 5: Split RAG Tests By Component Boundary

**Files:**
- Create: `tests/test_rag_schema.py`
- Create: `tests/test_rag_parsing.py`
- Modify: `tests/test_rag_store.py`

**Interfaces:**
- Schema tests use `apply_migration` and `connect` directly.
- Parsing tests import parsed records and `load_rag_file` through the public `hieronymus.rag` facade.
- Store tests import only `RagStore`, config, registry, and path/test helpers.

- [x] **Step 1: Move schema tests and their SQL helpers**

Move `_insert_series`, `_insert_rag_source`, `_insert_rag_chunk`,
`_fts_match_count`, and these tests to `test_rag_schema.py`:

```text
test_rag_schema_is_created_idempotently
test_rag_chunk_series_must_match_source_series
test_rag_chunk_fts_triggers_sync_insert_update_and_delete
```

- [x] **Step 2: Move parsing/model tests**

Move `test_rag_dataclasses_expose_payload_fields` and all tests from
`test_text_file_is_chunked_by_paragraph` through
`test_yaml_file_accepts_mapping_entries` to `test_rag_parsing.py`, including the
new invalid-header parameterization. Keep their assertions unchanged.

- [x] **Step 3: Remove unused imports and keep store helpers local**

The remaining `test_rag_store.py` begins with `_series` and imports only:

```python
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.rag import RagStore
from hieronymus.registry import Registry
```

- [x] **Step 4: Run the three component suites**

Run:

```bash
uv run pytest tests/test_rag_schema.py tests/test_rag_parsing.py tests/test_rag_store.py -q
uv run ruff check tests/test_rag_schema.py tests/test_rag_parsing.py tests/test_rag_store.py
```

Expected: all moved tests pass and collection has no duplicate test names.

- [x] **Step 5: Commit the test split**

```bash
git add tests/test_rag_schema.py tests/test_rag_parsing.py tests/test_rag_store.py
git commit -m "test: split rag coverage by component"
```

## Task 6: Complete Payload Coverage And Original Plan Status

**Files:**
- Modify: `tests/test_recall_enriched_memory.py`
- Modify: `docs/superpowers/plans/2026-07-04-rag-pipelines.md`

**Interfaces:**
- Consumes: `RecallResult.rag(..., rank=1, score=0.75, ...)` and `enriched_payload()`.
- Produces: explicit regression assertions without changing the payload contract.

- [ ] **Step 1: Extend the existing payload test**

Add beside its existing assertions:

```python
    assert result.rank == 1
    assert payload["score"] == 0.75
```

- [ ] **Step 2: Run the payload test**

Run:

```bash
uv run pytest tests/test_recall_enriched_memory.py::test_rag_recall_result_enriched_payload_contains_citation_fields -q
```

Expected: PASS; this is coverage of already implemented behavior, not a production change.

- [ ] **Step 3: Confirm documentation content exists**

Run:

```bash
rg -n "Import Project RAG Sources|dual-lane strategy" docs/usage.md docs/memory-dreaming.md
```

Expected: both original Task 7 documentation additions are present.

- [ ] **Step 4: Run the original focused RAG verification**

```bash
uv run pytest tests/test_rag_schema.py tests/test_rag_parsing.py tests/test_rag_store.py tests/test_rag_cli.py tests/test_mcp_rag.py tests/test_recall_rag.py tests/test_recall_enriched_memory.py tests/test_combined_recall.py
```

Expected: PASS.

- [ ] **Step 5: Commit payload coverage**

```bash
git add tests/test_recall_enriched_memory.py
git commit -m "test: complete rag payload coverage"
```

## Task 7: Full Verification And Closure

**Files:**
- Modify: `docs/superpowers/plans/2026-07-04-rag-pipelines.md`
- Modify: `docs/superpowers/plans/2026-07-15-rag-pipelines-completion.md`

**Interfaces:** None; this task records verified completion only.

- [ ] **Step 1: Run the required full verification**

Run each command to completion:

```bash
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Expected: 0 test failures, Ruff reports no lint errors, and every file is formatted.

- [ ] **Step 2: Mark original Task 7 complete**

Change its five checkboxes to `[x]` only after Step 1 succeeds and confirm the
documentation commit `e9ba18d` remains in branch history.

- [ ] **Step 3: Mark this completion plan complete**

Change every successfully executed checkbox in this file to `[x]`.

- [ ] **Step 4: Re-run documentation and diff checks**

```bash
git diff --check
rg -n '^- \[ \]' docs/superpowers/plans/2026-07-04-rag-pipelines.md docs/superpowers/plans/2026-07-15-rag-pipelines-completion.md
git status --short
```

Expected: no whitespace errors, no unchecked plan steps, and only the intentional
plan-status edits remain unstaged.

- [ ] **Step 5: Commit verified plan closure**

```bash
git add docs/superpowers/plans/2026-07-04-rag-pipelines.md docs/superpowers/plans/2026-07-15-rag-pipelines-completion.md
git commit -m "docs: complete rag pipelines plan"
```
