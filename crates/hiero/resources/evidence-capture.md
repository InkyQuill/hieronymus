# Exact evidence capture

Use this example only when the project agreement permits capture. Discover the installed `hieronymus_evidence_capture` input schema through MCP `tools/list` first, including its snapshot, selection, binding and applicability definitions. The snippets below are templates, not ready-to-send JSON: replace uppercase unquoted tokens with observed JSON values, never guessed IDs.

Resolve SERIES_ID via `hieronymus_series_list`, CONCEPT_ID via concept lookup/create only when identity is supported, and POSITION_ID from the relevant `hieronymus_order_register` output. Copy APPLICABILITY from coherent observed evidence/context, including its actual chronology and knowledge gates; never invent `All` or resolved scope to run this example. If these prerequisites are unknown, keep an ordinary tentative observation instead. This example assumes registered source language `ja` and target language `ru`; use the actual project languages for other text.

## Source snapshot and selection

For an exact file containing `猫と犬` with no newline, 犬 occupies the zero-based, end-exclusive UTF-8 byte interval [6,9). Character indices [2,3) are not valid byte coordinates. Both endpoints must be UTF-8 character boundaries. Compute SOURCE_SHA256 over **all exact file bytes**, not just 犬. SOURCE_PATH is the actual absolute path, encoded as a JSON string; SOURCE_SHA256 is a JSON string containing the hexadecimal digest. Preserve BOMs, CRLF, whitespace and Unicode normalization as found. Never normalize text or change `expected_text` to force a capture to pass.

Call `hieronymus_evidence_capture` with:

```json
{
  "series_id": SERIES_ID,
  "snapshot": {"kind": "file", "path": SOURCE_PATH, "expected_hash": SOURCE_SHA256},
  "kind": "source_passage",
  "selection": {"start": 6, "end": 9, "expected_text": "犬"},
  "binding": {
    "concept_id": CONCEPT_ID,
    "source_language": "ja",
    "target_language": "ru",
    "applicability": APPLICABILITY,
    "position_id": POSITION_ID,
    "paragraph_start": 0,
    "paragraph_end": 0,
    "identity_anchor": true,
    "aligned_source_id": null,
    "rendering": null,
    "contradicts_rule": null,
    "contradicts_claim": null,
    "claim_effect": null,
    "conflict_kind": null
  }
}
```

The server derives paragraph bounds; the two zero fields are input placeholders, not asserted paragraph coordinates. `identity_anchor:true` is appropriate only for the resolved concept demonstrated by the selected passage. Verify `selected_text` and preserve the returned `reference` unchanged, including its kind, ID, content_hash and byte span. Set SOURCE_EVIDENCE_ID to that reference's ID. A hash mismatch or text/boundary error requires inspecting the current file and recomputing a coherent snapshot/selection, not bypassing validation.

## Target snapshot and alignment

For an exact target file containing `Кот и пёс` with no newline, пёс occupies [10,16) in UTF-8 bytes. Compute TARGET_SHA256 over the entire target file. Capture the alignment with the same observed concept, applicability and story position:

```json
{
  "series_id": SERIES_ID,
  "snapshot": {"kind": "file", "path": TARGET_PATH, "expected_hash": TARGET_SHA256},
  "kind": "aligned_rendering",
  "selection": {"start": 10, "end": 16, "expected_text": "пёс"},
  "binding": {
    "concept_id": CONCEPT_ID,
    "source_language": "ja",
    "target_language": "ru",
    "applicability": APPLICABILITY,
    "position_id": POSITION_ID,
    "paragraph_start": 0,
    "paragraph_end": 0,
    "identity_anchor": true,
    "aligned_source_id": SOURCE_EVIDENCE_ID,
    "rendering": null,
    "contradicts_rule": null,
    "contradicts_claim": null,
    "claim_effect": null,
    "conflict_kind": null
  }
}
```

The selection indexes the complete **target** snapshot. `aligned_source_id` links source evidence; it does not switch coordinates to the source file. The server inherits source paragraph anchors and derives rendering from the exact selected target text. Preserve both returned references in a decision draft's `evidence_refs` when needed. One pair alone does not establish learned terminology activation.

## Optional future-prompt binding

Only when prompt capture is intended, reuse the leading task's compatible active session and read `hieronymus_recall` in that session. Call `hieronymus_session_start` only when a standalone task has no session; retain its returned ID and complete it when that task ends. Do not create a second session or replace the caller's binding for nested evidence work. Copy its `resulting_revision` as EXPECTED_REVISION after both captures; selections must still be coherent. SOURCE_REFERENCE below is the complete observed source reference, not the aligned-rendering reference. SESSION_ID is the observed memory-session ID; HOST_SESSION_ID is the real host hook identity, a JSON string. Inspect `hiero agent-hook bind-context --help` for the installed schema, then provide this object on stdin (using the actual host):

```json
{
  "version": 1,
  "host": "codex",
  "host_session_id": HOST_SESSION_ID,
  "series_id": SERIES_ID,
  "session_id": SESSION_ID,
  "expected_revision": EXPECTED_REVISION,
  "source_language": "ja",
  "target_language": "ru",
  "applicability": APPLICABILITY,
  "selected_sources": [SOURCE_REFERENCE],
  "selected_claims": [],
  "selected_rule": null
}
```

Binding permits future capture and confers no authority. Read bootstrap's Jev disclosure before enabling it. Do not replay the current prompt, invent a host event, or rebase a stale delivery. Keep aligned evidence in the decision draft, never in `selected_sources`.
