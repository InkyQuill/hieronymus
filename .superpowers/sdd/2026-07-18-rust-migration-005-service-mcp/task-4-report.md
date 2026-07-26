# Task 4 Report: Register and Test All MCP Tools

## Outcome

Implemented the native Rust MCP surface with rmcp `=2.2.0`, the official stateful
`StreamableHttpService`, and `LocalSessionManager`. `/mcp` now serves the MCP protocol
behind the daemon's existing token, Host, and Origin middleware. `/api/mcp/{operation}`
remains the Task 5 placeholder.

The handlers dispatch through the real registry, concept, crystal, termbase, workspace,
recall, feedback, RAG, and Dream services. Dream execution is behind an injected
`DreamRunner` boundary; production loads the provider catalog/registry and calls
`DreamService`, while tests inject a deterministic provider without external network access.

## TDD Evidence

### MCP transport and catalog RED

```text
$ cargo test -p hiero-bin --test test_mcp http
error[E0432]: unresolved import `hiero_bin::mcp`
test failed to compile
```

After adding the rmcp transport and session lifecycle:

```text
$ cargo test -p hiero-bin --test test_mcp http
running 2 tests
test http_initialize_creates_session_then_lists_and_calls_tools ... ok
test http_rejects_unknown_tool_invalid_payload_and_foreign_origin ... ok
test result: ok. 2 passed; 0 failed
```

### Typed concept operations RED/GREEN

The preserved Python schemas exposed concept and facet fields that Phase 003 did not yet
offer as public store inputs. Direct SQL in the MCP adapter was avoided by adding narrow,
transactional core APIs.

```text
$ cargo test -p hiero-core --test test_concepts typed_updates
error[E0432]: unresolved imports `AddConceptFacetInput`,
`UpdateConceptFacetInput`, `UpdateConceptInput`
error[E0599]: no method named `update` / `add_facet_typed` / `update_facet_typed`
```

```text
$ cargo test -p hiero-core --test test_concepts typed_updates
running 2 tests
test typed_updates_persist_all_mcp_concept_and_facet_fields ... ok
test typed_updates_reject_invalid_status_confidence_and_missing_source_crystal ... ok
test result: ok. 2 passed; 0 failed
```

Rename provenance received its own RED/GREEN cycle:

```text
RED: no method named `rename_concept_with_source`
GREEN: 1 passed; 0 failed
```

### Crystal relationship confidence RED/GREEN

```text
RED: no methods named `set_story_scopes_with_confidence` and
`set_semantic_tags_with_confidence`
GREEN: explicit_side_table_confidence_is_persisted ... ok
```

### Term notes RED/GREEN

```text
RED: left "", right "approved reader-facing spelling"
GREEN: store_backend_dispatches_registry_concept_rule_workspace_termbase_rag_and_feedback ... ok
```

### Final focused MCP result

```text
$ cargo test -p hiero-bin --test test_mcp
running 8 tests
test catalog_registers_the_exact_forty_unique_tool_names ... ok
test every_registered_tool_has_a_stable_object_schema ... ok
test http_initialize_creates_session_then_lists_and_calls_tools ... ok
test http_rejects_unknown_tool_invalid_payload_and_foreign_origin ... ok
test http_surfaces_backend_errors_as_tool_errors ... ok
test store_backend_dispatches_registry_concept_rule_workspace_termbase_rag_and_feedback ... ok
test production_dream_runner_executes_dream_service_with_an_injected_provider ... ok
test production_dream_runner_reports_invalid_configuration ... ok
test result: ok. 8 passed; 0 failed
```

## Exact Tool Catalog

The expected-name test proves 40 unique registrations:

1. `hieronymus_concept_archive`
2. `hieronymus_concept_create`
3. `hieronymus_concept_facet_add`
4. `hieronymus_concept_facet_list`
5. `hieronymus_concept_facet_set_canonical`
6. `hieronymus_concept_facet_update`
7. `hieronymus_concept_get`
8. `hieronymus_concept_list`
9. `hieronymus_concept_merge`
10. `hieronymus_concept_proposals_list`
11. `hieronymus_concept_rename`
12. `hieronymus_concept_semantic_tags_set`
13. `hieronymus_concept_update`
14. `hieronymus_crystal_link_concept`
15. `hieronymus_crystal_semantic_tags_set`
16. `hieronymus_crystal_story_scopes_set`
17. `hieronymus_dream`
18. `hieronymus_feedback`
19. `hieronymus_memory_add`
20. `hieronymus_memory_search`
21. `hieronymus_rag_import`
22. `hieronymus_rag_search`
23. `hieronymus_recall`
24. `hieronymus_recall_feedback`
25. `hieronymus_rule_crystal_archive`
26. `hieronymus_rule_crystal_validate`
27. `hieronymus_rule_crystals_list`
28. `hieronymus_series_create`
29. `hieronymus_series_init`
30. `hieronymus_series_list`
31. `hieronymus_series_set_language_tags`
32. `hieronymus_session_complete`
33. `hieronymus_session_start`
34. `hieronymus_short_term_add`
35. `hieronymus_short_term_add_batch`
36. `hieronymus_status`
37. `hieronymus_termbase_approve`
38. `hieronymus_termbase_contract`
39. `hieronymus_termbase_propose`
40. `hieronymus_termbase_validate`

The singular Python names `hieronymus_rule_crystal_archive` and
`hieronymus_rule_crystal_validate` are preserved. The new fortieth tool is
`hieronymus_recall_feedback`.

## Schema and Protocol Coverage

- Every input type derives both serde `Deserialize` and schemars `JsonSchema`.
- The same type is used to produce the advertised schema and decode dispatch arguments.
- The complete catalog snapshot is
  `crates/hiero-bin/tests/snapshots/test_mcp__all_mcp_tool_schemas.snap`.
- Tests cover initialize, returned session ID, stateful `tools/list`, `tools/call`,
  unknown tool, invalid payload, backend error, missing Origin, same-origin handling,
  foreign-Origin rejection, and the existing token middleware.
- Authorized empty `/mcp` POSTs now reach rmcp and return its protocol-level `406`,
  replacing the old placeholder `501`; service route tests were updated accordingly.

## Store Coverage

- Registry: create/list/language-tag dispatch.
- Concepts/facets: full metadata, canonical selection, tags, provenance, archive/merge/rename.
- Crystals/rules: concept links, scoped metadata with supplied confidence, non-empty
  `rule_intent` filtering, validation, archive.
- Termbase: complete series/language/story `TranslationContext`, propose notes persistence,
  approve, contract, validation.
- Workspace: sessions, short-term single/batch writes, correction feedback.
- Recall: real recall activation plus `FeedbackStore::record_recall_outcome` for
  `{useful, miss}`.
- RAG: real local file import and lexical search path.
- Dream: the production runner loads the configured provider catalog/registry and dispatches
  to `DreamService`; deterministic provider injection and invalid configuration are tested
  without an external provider call.

## Verification

```text
$ cargo fmt --all --check
exit 0

$ cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
Finished `dev` profile
exit 0

$ cargo test --workspace --all-features --locked
all unit, integration, snapshot, command, and doc tests passed
exit 0

$ cargo doc --workspace --no-deps --all-features --locked
Generated target/doc/hiero_bin/index.html and 2 other files
exit 0

$ git diff --check
exit 0
```

## Files Changed

- `Cargo.toml`, `Cargo.lock`, `crates/hiero-bin/Cargo.toml`
- `crates/hiero-bin/src/mcp/{mod,backend,http,tools}.rs`
- `crates/hiero-bin/src/{daemon,lib}.rs`
- `crates/hiero-bin/tests/{test_mcp,test_service}.rs`
- `crates/hiero-bin/tests/snapshots/test_mcp__all_mcp_tool_schemas.snap`
- `crates/hiero-core/src/domain/{concepts,crystals,models,mod,termbase}.rs`
- `crates/hiero-core/tests/{test_concepts,test_crystals,test_ingest,test_termbase}.rs`

## Self-review

- No custom JSON-over-HTTP protocol was added; all MCP framing and session behavior is rmcp.
- No handler bypasses the core stores with adapter-owned persistence.
- Supplied concept provenance, facet/crystal confidence, and term notes are retained.
- Rule listing explicitly requires non-empty `rule_intent`.
- Tool/backend failures are returned as MCP tool errors; unknown tools use the protocol method
  error.
- The daemon injects the production `StoreDreamRunner`; provider selection uses an explicit MCP
  override when supplied, otherwise the configured workflow provider, and configuration/provider
  failures surface as MCP tool errors.

## Fix Round 1

Addressed all nine Important review findings without changing the rmcp 2.2 stateful transport:

1. MCP tool failures now log the complete error chain server-side and return the stable
   `The tool could not complete the request.` message. Only `SafeMcpError` payload-validation
   failures retain classified client-safe detail. Transport tests prove API secrets, private
   paths, SQL details, provider bodies, and a real closed-database error are redacted.
2. An unscoped concept list now uses bounded `ConceptStore::list_all`, returning global and
   series-scoped concepts in stable id order.
3. Dedicated concept, facet, and crystal payload builders preserve the established flat Python
   response contracts, including `semantic_tags`, facet `kind`, normalized side metadata, and
   crystal concept ids. Literal JSON tests cover concept create/get/update, facet add/update, and
   all three crystal mutation tools.
4. `ConceptStore::create_primitive` validates and persists all concept fields and semantic-tag
   side rows under one `BEGIN IMMEDIATE` transaction. An injected late tag trigger proves total
   rollback; the MCP adapter calls only this boundary.
5. Generic concept create/update accepts only `candidate` and `established`; `archived` remains
   reachable solely through the dedicated archive operation. Core tests cover both rejection
   paths and successful archival.
6. Term approval loads `Termbase::candidate_context` first and derives the authoritative
   series/language pair and persisted context metadata. The approve schema no longer accepts
   caller-supplied languages; a non-default Korean-to-German proposal approves under a
   Japanese-to-Russian series default.
7. MCP status and HTTP status now share `api::system::status_report`, including the live database
   probe and doctor report. Invalid configuration produces an actual degraded doctor check.
8. `WorkspaceStore::get_or_start_default_session` atomically finds or inserts the matching active
   default session. Repeated legacy memory add/search calls share one session and one context.
9. All tool descriptions now match the Python registrations exactly. The compatibility warning is
   restored for every decorated legacy wrapper, and descriptions are snapshotted alongside input
   schemas.

### Fix-round RED evidence

```text
concept mixed-scope RED:
called `Option::unwrap()` on a `None` value

flat response RED:
left contained timestamps and omitted semantic_tags
right was the established flat MCP object

term approval RED:
term approve conflicted with existing state:
term series and language pair do not match the termbase context

status RED:
left: null
right: true

sanitization RED:
left: "secret API key sk-super-secret"
right: "The tool could not complete the request."

core atomic API RED:
unresolved import `CreateConceptPrimitiveInput`
no method named `create_primitive` or `list_all`

default-session RED:
no method named `get_or_start_default_session`

candidate-context RED:
no associated item named `candidate_context`
```

### Fix-round GREEN evidence

```text
$ cargo test -p hiero-bin --test test_mcp
running 13 tests
test result: ok. 13 passed; 0 failed

$ cargo test -p hiero-core --test test_concepts
running 12 tests
test result: ok. 12 passed; 0 failed

$ cargo test -p hiero-core --test test_memory
running 20 tests
test result: ok. 20 passed; 0 failed

$ cargo test -p hiero-core --test test_termbase
running 13 tests
test result: ok. 13 passed; 0 failed
```

### Fix-round final gates

```text
$ cargo fmt --all --check
exit 0

$ cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
Finished `dev` profile
exit 0

$ cargo test --workspace --all-features --locked
all unit, integration, snapshot, command, and doc tests passed
exit 0

$ cargo doc --workspace --no-deps --all-features --locked
Generated target/doc/hiero_bin/index.html and 2 other files
exit 0

$ git diff --check
exit 0
```
