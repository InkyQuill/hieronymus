# Product and Release Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish ADR 0016's autonomous-memory policy and realistic retrieval/host acceptance, then complete F1/F2 using the repaired runtime.

**Architecture:** Keep SQLite structured authority separate from graded retrieval. Corrections and automatic decisions enter validated, transactional domain operations; plugins teach the ordinary autonomous loop. Release acceptance exercises the installed application rather than only in-process stores.

**Tech Stack:** Existing Rust 1.96 / SQLite / LanceDB / ort / tokenizers; Svelte 5; Bun 1.4.0; Linux x86_64. Preserve current dependency pins until a measured retrieval requirement warrants a separately qualified change.

**Spec:** [Implementation review](../../astra-implementation-review-2026-09-06.md), [ADR 0016](../../adr/0016-autonomous-story-memory-product-vision.md), [correctness prerequisites](2026-09-06-merged-port-correctness.md), [original F1/F2 plan](2026-09-05-rust-port-release.md).

## Global Constraints

- Both working memory and semantic RAG are mandatory; no FTS-only release alternative.
- ADR 0016 supersedes older plans' human-only rule lifecycle instructions. Preserve deterministic enforcement, explicit-user priority, evidence, revision checks and audit.
- No mandatory human approval inbox, author tagging/scoring, or recurring curation step.
- Linux `x86_64-unknown-linux-gnu`; MCP revision `2026-07-28`; preserve frozen historical fixtures and record intentional Rust deltas separately.
- Configuration, host capabilities and semantic failures must be reported honestly. Do not claim a host or language is supported from synthetic or adjacent tests.
- F1/F2 remain unimplemented. This plan neither credits them as done nor expands them into a new attestation platform.
- Preserve existing dirty ADR/product documents; write new focused artifacts. No live migration, publication or user-host configuration rewrite during tests.

---

## File structure and order

P1 creates the focused autonomous-authority design and its implementation breakdown; it is design work, not an excuse to remove guards immediately. P2 completes measured retrieval acceptance. P3 addresses the observed acquisition prerequisite and executes the existing F1/F2 tasks with the corrections below. Execute correctness C1–C9 before F1/F2 acceptance; P1 and corpus preparation may proceed earlier.

## Task P1: Specify autonomous authority, corrections and story applicability

**Coverage:** ADR 0016's new requirements; obsolete human-only generator guidance. These were not part of the agents' original completion obligations.

**Files:**

- Create: `docs/superpowers/specs/2026-09-06-autonomous-authority-and-corrections.md`
- Create: `docs/superpowers/plans/2026-09-06-autonomous-authority-implementation.md`
- Reference: `docs/adr/0016-autonomous-story-memory-product-vision.md`
- Reference: `crates/hieronymus/src/terminology.rs`, `crates/hieronymus/src/feedback.rs`, `crates/hieronymus/src/dream_output.rs`, `crates/hieronymus/src/recall.rs`
- Reference: `crates/hiero/src/agent_plugins.rs`, `src/hieronymus/agent_assets.py`

**Interfaces to specify:** A versioned structured decision request carrying `decision_id`, `expected_revision`, `actor_kind`, `origin`, `evidence_refs`, `concept_id`, source/target language, story applicability and requested operation. Distinguish `learned` from `explicit_user` authority; origins must be attached at a trustworthy ingestion boundary rather than accepted as arbitrary provider prose. The design must define typed request/result/error variants and exact SQLite migration ownership before coding.

- [ ] Read the accepted ADR and map all seven product acceptance scenarios to current entry points and missing behavior. Preserve the old implementation grades: this is new product work.
- [ ] Write the decision table: learned inference may revise learned rules with sufficient contextual evidence; explicit user instructions win within scope; passive score/negative-usefulness feedback cannot revoke a hard rule; a later explicit correction may replace it. Require exact concept/language/applicability resolution and revision checking. Specify measurable evidence criteria; do not use a provider's confidence number alone as authority.
- [ ] Write the correction protocol with three distinct intents: factual invalidation/qualification, relevance feedback, and rendering instruction. A clear rendering reaches the structured contract before dependent validation succeeds; a clear factual invalidation immediately suppresses or qualifies the obsolete recall hit. Persist the immediate effect and asynchronous consolidation intent in one transaction. Define replay, concurrent corrections and failure behavior.
- [ ] Specify story position and viewpoint applicability: earlier/later validity, character knowledge versus narrator knowledge, evolution versus correction, and uncertain identity. A later revelation must not be offered as already known in an earlier scene. Resolve exact representation against existing volume/chapter/story scopes; include examples for nonnumeric chapter identifiers rather than silently imposing numeric ordering.
- [ ] Specify plugin behavior and packaging: automatic recall/capture/correction in normal work, no mandatory human approval language; Claude/zCode share the Claude-format bundle; Codex uses its own supported integration. Hooks are optional enhancements, not required correctness mechanisms. No copied backend and no automatic changes to book content.
- [ ] Write the implementation plan from that concrete design, with exact migrations, domain APIs, plugin files and failure-path tests. Do not close P1 merely with another list of unanswered design topics. Explicitly record any unresolved external requirement; continue independently resolvable design work.

Minimum acceptance examples carried into the implementation plan:

| Given / operation | Required immediate and durable result |
| --- | --- |
| Learned rule says A; user clearly instructs B for this concept/scope | Next contract/validation uses B; restart preserves B; later Dream preference for A cannot undo B |
| User says a remembered event is wrong, without a replacement | Next recall suppresses or qualifies that claim; it does not invent a replacement |
| User says a result was unhelpful | Relevance changes; the underlying fact is not automatically declared false |
| Two distinct characters share a surface name | Ambiguity stays tentative; no global string replacement or mandatory human inbox |
| New evidence revises a learned rule | Structured validation, expected revision, evidence and audit commit with the replacement |
| Revelation at chapter 12; query from chapter 3 character viewpoint | Later knowledge is excluded or explicitly marked future/outside-viewpoint |
| Provider unavailable during correction | Clear immediate correction remains effective; consolidation retries boundedly without losing it |

P1 is documentation-first design work, so no artificial tests asserting Markdown wording are required. Review the decision table against ADR 0016, then commit only the two new design/plan artifacts. Do not implement autonomous mutation by deleting active-rule checks before the replacement path exists.

## Task P2: Complete realistic retrieval and host-workflow acceptance

**Coverage:** A14, memory/RAG requirement, ADR 0016 workflow/temporal acceptance.

**Dependencies:** C3–C6 for semantic truth; P1 for correction and temporal expected behavior.

**Files:**

- Modify: `crates/hiero/tests/fixtures/hybrid-relevance.json`
- Modify: `crates/hiero/tests/semantic_real.rs`
- Modify: `docs/semantic-validation.md`
- Create: `docs/agent-host-acceptance.md`
- Modify when measured results require it: `crates/hieronymus/src/semantic_model.rs`, `crates/hieronymus/src/semantic_tokenizer.rs`, `crates/hieronymus/src/semantic_embeddings.rs`

**Interfaces:** Keep the existing corpus fields and normal MCP operations. Add named acceptance cases with language, context/viewpoint, expected source IDs and excluded IDs; the test must distinguish model retrieval from deterministic contract and lexical short-term memory. Model/tokenizer changes create a new identity and rebuild; never relabel old vectors.

- [ ] Add real source/target-language paraphrases to the committed corpus, with manually specified expected IDs and distractors. Begin with the actual supported literary languages; the following Japanese pair is a concrete first regression if Japanese is in the declared target set:

```json
{
  "document": "船医は負傷した船乗りを手当てした。",
  "query": "けがをした水夫を治療したのは誰？",
  "expected_source_id": "ship-physician-ja",
  "distractor": "大工は船室の扉を修理した。"
}
```

- [ ] Run the existing real ONNX suite using explicit checksum-verified runtime/model paths. Record failures without replacing the queries with exact lexical matches. Extend the fixture DTO explicitly for the new context fields; no silent ignored fields.
- [ ] Exercise both `hieronymus_recall` and `hieronymus_rag_search`, repeated sessions, lexical and semantic-only evidence, corrections, cross-series distractors and earlier/later story applicability. Once P1's implementation exists, require its immediate-correction and viewpoint expectations here. Do not claim those behaviors from a storage-only test.
- [ ] If the pinned model fails required language cases, qualify a replacement model/tokenizer/runtime combination in a focused change. Compare the same corpus, latency, memory and installed artifact size; record the new hashes and generation invalidation. Do not guess a replacement based on popularity or weaken the language scope merely to preserve green tests.
- [ ] Install generated bundles into disposable Claude/zCode and Codex configurations where those hosts are available. Run ordinary Read/Learn/Remember/correction workflows with MCP access, including a host without hooks. Record unavailable hosts as unverified, not passed. Keep user configuration untouched.
- [ ] Record exact commands, asset hashes, all corpus outcomes, host versions/capabilities and remaining limits in the two evidence documents; commit only the verified test/model/document changes.

Required assertions in the real test loop, using its existing result values:

```rust
assert!(top_three_source_ids.contains(&expected_source_id));
assert!(returned_series.iter().all(|series| series == &requested_series));
assert!(semantic_only_result_reasons.iter().any(|reason| reason == "rag semantic match"));
```

Define the three collections directly from the normal MCP payload rows in the test, with `expected_source_id` and `requested_series` from the fixture. These are behavioral assertions, not a prescribed arbitrary test count. Missing required assets must fail the explicitly requested live suite.

## Task P3: Restore acquisition reproducibility, then execute F1/F2

**Coverage:** A15; F1/F2 explicitly not yet implemented.

**Dependencies:** C1–C9, P2 semantic acceptance. ADR 0016 product cutover additionally requires the implemented P1 contract, not only its design.

**Files:**

- Inspect/fix as reproduced: `tools/qualification/acquire.py`
- Modify: `tests/qualification/test_acquire.py`
- Create: `docs/rust-cutover-rehearsal.md`
- Execute the exact F1/F2 file lists in `2026-09-05-rust-port-release.md`, preserving unrelated edits.

**Interfaces:** The existing acquisition commands must verify the pinned archive and extraction and return a committed usable path. The release consumes verified runtime/model/tokenizer assets; its acceptance evidence may link the semantic report if the existing consumer supports that. Do not assume a machine-record schema expansion is necessary without reading that consumer.

- [ ] Reproduce the observed failure with `uv run python -m tools.qualification.acquire onnx-runtime`. Inspect the approved archive digest, extracted-tree digest and provenance inclusion/exclusion. Add a regression using the existing acquisition test fixtures that makes a valid approved archive pass and a modified extracted file fail. Do not bypass or weaken the checksum/tree-validation contract.
- [ ] Fix the specific reproducible tree-validation discrepancy and run `uv run pytest tests/qualification/test_acquire.py`, then the real acquisition command. The review's independently checksum-verified temporary library is evidence for the semantic rerun, not a substitute for a successful installer/acquirer.
- [ ] Read the F1/F2 plan and apply these precedence corrections: ADR 0016 replaces its historical human-only global constraint; semantic readiness uses C3/C4's verified service state; acquisition must pass normally; proposal inspection is optional diagnostics rather than a required author-curation workflow.
- [ ] Execute F1: package the real runtime/model/tokenizer and licenses, validate all native dependencies including tokenizers/onig, support verified local/remote staging, and test failures before activation. No required developer `LD_LIBRARY_PATH`, Python, Node or Bun on the target.
- [ ] Execute F2: real installed CLI/MCP, real browser bootstrap and all applicable actions, upgrade/recovery/rollback, SIGTERM, required semantic relevance, supported host workflows and ADR 0016 acceptance. Disposable roots only. Correct the 16 rustdoc warnings and contributor/usage drift as part of this documentation gate.
- [ ] Validate release evidence against actual tooling consumers. If a schema expansion is required, make a small versioned extension with validator tests; otherwise link reproducible evidence. Do not recreate removed attestation/fixed-count gates.
- [ ] Commit the bounded acquisition fix separately from F1 and F2. Record what ran and any unverified requirement; do not declare release-ready while mandatory semantic, host or autonomous-policy acceptance is absent.

Final commands include:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked --no-fail-fast
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

Run frontend typecheck/tests/build from `frontend`, the explicit live semantic suite with verified assets, and the installed/browser/host rehearsal separately. Default green tests do not execute ignored cases or verify an installed application.

## Plan review and handoff

This plan covers the new product direction, realistic retrieval limits and unimplemented release work without retroactively lowering the implementation grades. The companion correctness plan covers A1–A13. Execute the existing F tasks only after their prerequisites are real; publication and live-data cutover remain separate explicit actions.
