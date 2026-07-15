# RAG Pipelines Completion Design

## Context

The RAG pipelines MVP is implemented and its focused tests pass, but the original
implementation plan is not formally complete. Its final documentation and full
verification task remains unchecked. Review also found two import edge cases and
two files whose mixed responsibilities conflict with the project's preference for
small modules with explicit boundaries.

This completion pass must preserve the existing public RAG API and the approved
MVP behavior while resolving those gaps. It must not add embeddings, directory
import, admin screens, or any other RAG backlog feature.

## Goals

- Reject CSV and TSV glossaries with blank or duplicate normalized headers.
- Reindex a source when its parsed source type or content type changes, even when
  the bytes and checksum are unchanged.
- Preserve the existing `hieronymus.rag` imports used by application code and
  external callers.
- Separate parsing and chunking from persistence, search, and ranking.
- Split RAG tests along the same component boundaries.
- Complete the original RAG plan's documentation and verification task with
  current evidence.
- Prevent the full test suite from invoking the real installed Hieronymus tool
  during uninstall-script tests.

## Architecture

`hieronymus.rag` remains the public compatibility facade. Existing imports of
`RagStore`, `load_rag_file`, `ParsedRagFile`, and `ParsedRagChunk` continue to
work without caller changes.

The implementation is divided into two focused modules:

- `rag_parsing.py` owns supported file types, checksum calculation, text and
  Markdown chunking, glossary parsing, and parsed-file dataclasses.
- `rag_store.py` owns `RagStore`, SQLite row hydration and mutation, metadata tag
  persistence, FTS retrieval, scoring, and rank reasons.

`rag.py` only re-exports the stable public API. The model records in
`rag_models.py` remain unchanged unless typing imports require adjustment.

Tests mirror these boundaries:

- schema and database invariants;
- parsing, chunking, and parsed dataclasses;
- store import, replacement, retrieval, and ranking;
- existing CLI, MCP, and recall integration suites.

## Import Validation

Delimited glossaries normalize each header with `strip()` before parsing rows.
Import fails with a clear `ValueError` if:

- there is no header row;
- any normalized header is empty;
- two normalized headers are equal.

Duplicate detection therefore treats `source` and ` source ` as the same field.
Validation happens during parsing, before any database mutation, so a failed
changed import preserves the previously indexed source.

The unchanged-source fast path requires equality of checksum, `source_type`, and
`content_type`. If the same `series_slug + source_ref` is imported with identical
bytes under another supported format, normal replacement runs and refreshes the
stored metadata and chunks. Tag-only changes still refresh chunk side tables and
return `skipped=True` when all three identity fields match.

## Full-Suite Isolation

Uninstall-script tests must never call the real `uv tool uninstall hieronymus`.
Their temporary `PATH` receives a deterministic fake `uv` executable, matching
the isolation already used by install-script tests. Tests continue to exercise
path validation, data retention, and removal behavior without mutating the
developer's installed tools or triggering network-dependent hooks.

## Testing Strategy

Behavior changes use red-green TDD:

1. Add separate failing tests for blank and duplicate delimited headers.
2. Add a failing regression test for identical bytes reimported through a
   different supported extension under the same source reference.
3. Add a failing isolation test proving uninstall execution uses the fake `uv`.
4. Implement the smallest fixes and verify each focused test turns green.
5. Move code and tests without behavior changes, running focused suites after
   each boundary extraction.
6. Extend the enriched RAG payload test to assert the payload `score` and the
   result object's `rank`. Rank is added by the CLI and MCP wrappers rather than
   `enriched_payload`, so no new enriched-payload field is introduced.
7. Run the original focused RAG command followed by the required project checks:
   `uv run pytest`, `uv run ruff check .`, and `uv run ruff format --check .`.

The original Task 7 checkboxes are marked complete only after their corresponding
documentation, commit, and verification evidence exists.

## Compatibility And Error Handling

No CLI or MCP signatures change. Existing source and chunk payloads remain
compatible. New validation errors are wrapped by `RagStore.import_file` as
`invalid RAG source: ...`, consistent with current parser failures.

Module extraction must not alter ranking weights, FTS queries, recall budgets,
rule-crystal protection, transaction boundaries, or activation behavior.
