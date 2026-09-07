# Rust Port Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close every confirmed gap in the Astra and Sonnet reviews and deliver working memory, semantic RAG, configured Dream execution, agent tools, and the web console.

**Architecture:** Keep the existing Rust domain stores, SQLite authority, derived LanceDB generations, blocking daemon, and Svelte console. Add one daemon application boundary, shared data-root ownership, and supervised workers; complete the missing integrations and durability paths instead of performing another rewrite.

**Tech Stack:** Rust 1.96, edition 2024; rusqlite 0.40.2; LanceDB 0.37.1; ort 2.0.0-rc.13; Arrow 58.3.0; Svelte 5; Bun 1.4.0. S1 adds pinned tokenizers 0.23.2 and qualifies its dependency closure.

**Spec:** [Astra's reconciled report](../../astra-report.md), [Sonnet's independent report](../../sonnet-report.md), all [accepted ADRs](../../adr), and the subsystem specifications linked below. Review baseline: `fcab0b5e4dea1e13a3b68884d441fb894c0e0417` on `feat/rust-rewrite-proposals`. These are proposed implementation steps; no implementation has been performed by writing this plan.

## Global Constraints

- **Owner decision, 2026-09-05: the app must deliver both memory and semantic RAG. Do not use ADR 0013's FTS-only allowance as a release or completion alternative.** FTS remains one hybrid-retrieval input. Real tokenization, inference, durable indexing, recall wiring and relevance evidence are mandatory.
- Accepted ADRs outrank implementation specs, fixtures, tests and Python behavior (ADR 0008). The missing deterministic contract and per-workflow provider/model selection are implementation obligations, not invitations to weaken ADRs.
- “All domain mutations used by normal CLI, MCP, and frontend workflows go through the daemon.” (ADR 0009)
- “The first Rust cutover supports `x86_64-unknown-linux-gnu`.” (ADR 0013); do not add macOS packaging to this program.
- “Only an authenticated user acting through the explicit rule-approval operation may transition `candidate` to `active`” (ADR 0011). Dream, feedback, decay and fuzzy recall cannot override active rules.
- “the separate CSRF token layer is waived.” (ADR 0012, 2026-09-03 amendment). Keep Host/Origin/session protection; do not restore waived CSRF or credentials_rotated behavior.
- MCP protocol revision remains `2026-07-28`; preserve frozen Python artifacts. Document ADR-backed Rust deltas in separate versioned expectations.
- Preserve the single data-root layout, plaintext local configuration policy, immutable backups and explicit migration/recovery boundaries.
- No end-user Python, Node or Bun runtime dependency. No Python runtime rollback. Do not refresh native dependency pins as unrelated cleanup.
- Do not introduce the fixed-count, attestation or additional gate tooling removed by the owner's ADR 0008 amendment. Record real outcomes and review the existing question list.
- Preserve unrelated work and Sonnet's report. Implementation tasks end in focused commits when executed; writing this plan does not authorize publication, live service replacement, or migration of the owner's data.

---

## File structure and plan boundaries

The scope spans independent subsystems, so this coordinator links six implementation plans. Read the relevant spec and dependency task before each task. Shared files are edited sequentially; the dependency order is not authorization for concurrent writers.

| Plan | Tasks | Deliverable |
| --- | --- | --- |
| [Runtime and upgrade](2026-09-05-rust-port-runtime.md) | R1–R5 | Read-only startup classification, exclusive ownership, ordered schema upgrades, truthful rollback, joined shutdown |
| [Application and agent tools](2026-09-05-rust-port-agent.md) | M1–M5 | All 39 MCP tools, independent rule contract, audited lifecycle, CLI and generated integrations |
| [Dream execution](2026-09-05-rust-port-dreaming.md) | D1–D5 | Per-workflow providers, complete outputs, atomic audit, durable pair progress, scheduler and draining |
| [Web console](2026-09-05-rust-port-console.md) | W1–W4 | Authenticated launch, ten views, thirteen actions, real provider checks, event resumption |
| [Memory and semantic RAG](2026-09-05-rust-port-semantic.md) | S1–S3 | Real tokenizer, supervised index jobs, live recall integration and real relevance evidence |
| [Distribution and rehearsal](2026-09-05-rust-port-release.md) | F1–F2 | Required semantic assets in the release, remote updater staging, installed-product verification |

Supporting specifications: [runtime](../specs/2026-08-31-rust-daemon-mcp-security-design.md), [terminology](../specs/2026-08-31-rust-terminology-memory-design.md), [compatibility](../specs/2026-08-31-rust-compatibility-contracts-design.md), [distribution](../specs/2026-08-31-rust-distribution-cutover-design.md). If an older spec conflicts with the explicit constraints above, follow the ADR/owner decision and record the expectation change.

## Execution order

Each checkbox represents the full task's RED → implementation → GREEN → review/commit cycle in its linked plan. Do not batch away its failure-path checks.

- [ ] R1 → R2 → R3: establish state classification, sole ownership and the ordered v2 upgrade before introducing new durable records.
- [ ] R5 → R4: make shutdown/readiness trustworthy before updater rollback uses it. R4 gains the required-semantic health assertion when S2 lands.
- [ ] M1 → M2 → M3 → M4: establish Application and complete domain adapters. M2 must explicitly report semantic-not-ready until S2; it cannot be declared final RAG completion.
- [ ] D1 → D2 → D3 → D4 → D5: resolve real workflows, cover their outputs, make completion atomic, retain budgeted work, then expose scheduling/draining.
- [ ] M5: connect remaining CLI/host workflows and execute the complete 39-tool matrix.
- [ ] S1 → S2 → S3: correct tokenizer identity, drive durable jobs in production and prove real memory/RAG retrieval. Re-run R4's candidate-health rejection with semantic failure.
- [ ] W1 → W2 → W3 → W4: complete browser launch, projections, actions, and refresh/reconnect.
- [ ] F1 → F2: package the actual required runtime/model/tokenizer, exercise update sources, then rehearse the installed product and align documentation.

This order intentionally places the semantic gate before release packaging acceptance. S1 can be implemented earlier after R1, and W1 after R5, but final completeness still requires every task.

## Cross-task interfaces

- R2 owns `RootOwnership` for the entire daemon or migrate/recover operation. R5 releases it only after all mutating workers have joined.
- R3 introduces schema v2 with `dream_link_batches/members/pairs` and `term_rule_actions`. D4 and M3 consume those tables through domain APIs. Capture the reviewed v1 schema first; never fabricate a “v1” fixture from a modified v2 create script.
- R5's `WorkerGroup` owns handles and a shared cancellation flag. D5's `DreamController` and S2's `SemanticController` register work with it. Controllers expose Send-capable message handles; non-Send providers/sessions are constructed and used inside their owning workers.
- M1's `Application::call(&self, name: &str, arguments: &Value, actor: &str) -> Result<Value,AppError>` is the shared authenticated tool boundary. It owns the live recall service and controller handles once D5/S2 land. Add an explicit construction seam for test provider injection, not production fixture host recognition.
- M2 returns `recall_id`, `deterministic_contract`, `results`, and `warnings`; the internal RecallResponse may retain `hits`. Rule contracts are independent of rankings, result limits and semantic availability.
- M3 owns explicit rule lifecycle. W3 and Dream graph actions cannot mutate active-rule projections directly; Dream has no approval authority.
- S1 supplies the same tokenizer/model identity to S2's indexing and query inference. R4/F1 must reject required-semantic readiness failures even when doctor returns a historically accepted degraded code.
- W1 is a deliberate bootstrap choice: a one-time 60-second grant travels in a fragment, which is removed before network/initial rendering. Never put installation bearer/session credentials in a URL or put grants in query strings. This follows ADR 0012; the earlier fragment-avoidance suggestion was not an ADR mandate.
- W4's event IDs are daemon-lifetime cursors; they are refresh hints, never substitutes for D3's durable audit.

## Complete advertised tool ownership

The exact advertised names come from `compatibility/snapshots/mcp.json`. M5 must execute all of them through real transports; comparing names alone is insufficient.

| Task | Tools |
| --- | --- |
| M1 | `hieronymus_status`, `hieronymus_series_create`, `hieronymus_series_init`, `hieronymus_series_list`, `hieronymus_series_set_language_tags`, `hieronymus_session_start`, `hieronymus_session_complete` |
| M2 | `hieronymus_memory_add`, `hieronymus_memory_search`, `hieronymus_short_term_add`, `hieronymus_short_term_add_batch`, `hieronymus_feedback`, `hieronymus_recall`, `hieronymus_rag_import`, `hieronymus_rag_search` |
| M3 | `hieronymus_termbase_propose`, `hieronymus_termbase_approve`, `hieronymus_termbase_contract`, `hieronymus_termbase_validate`, `hieronymus_rule_crystal_archive`, `hieronymus_rule_crystal_validate`, `hieronymus_rule_crystals_list` |
| M4 | `hieronymus_concept_create`, `hieronymus_concept_get`, `hieronymus_concept_list`, `hieronymus_concept_update`, `hieronymus_concept_archive`, `hieronymus_concept_merge`, `hieronymus_concept_rename`, `hieronymus_concept_semantic_tags_set`, `hieronymus_concept_facet_add`, `hieronymus_concept_facet_update`, `hieronymus_concept_facet_list`, `hieronymus_concept_facet_set_canonical`, `hieronymus_crystal_link_concept`, `hieronymus_crystal_story_scopes_set`, `hieronymus_crystal_semantic_tags_set`, `hieronymus_concept_proposals_list` |
| D5 | `hieronymus_dream` |

M4 store mapping: concept_create/get/list/update/archive/merge/rename → ConceptStore's corresponding create_concept/get/list_concepts/update/archive/merge/rename methods; concept_semantic_tags_set → set_semantic_tags; facet_add/list/set_canonical → add_facet/list_facets/set_canonical; facet_update → new update_facet; crystal_link_concept → link_crystal; crystal scope/tag setters → CrystalStore's existing setters; concept_proposals_list → new list_proposals. Use the actual store parameter types, and preserve the public tool's schema defaults in its adapter.

## Findings-to-task coverage

| Source finding | Remediation |
| --- | --- |
| Astra 1: unimplemented MCP dispatch | M1–M5, D5 |
| Astra 2: no console launch/auth bootstrap | R5, W1 |
| Astra 3: GET rejected without Origin | W1 |
| Astra 4: missing admin views/actions | W2–W3 |
| Astra 5: deterministic production Dream/no scheduler | D1, D5 |
| Astra 6: disabled workflows still execute | D1 |
| Astra 7: invented tokenizer IDs | S1, S3 |
| Astra 8: no production semantic executor/arming | S2–S3 |
| Astra 9: no exclusive root ownership | R2, R5 |
| Astra 10: updater health/rollback failures | R4, F1–F2; S2 required-semantic health |
| Astra 11: installation token rotates on restart | R5 |
| Astra 12: production fixture-host success | W3 |
| Astra 13: missing deterministic contract | M2–M3 |
| Astra 14: direct CLI feedback DB writes | M5 |
| Astra 15: malformed/partial startup accepted | R1 |
| Astra 16: config reads perform migration | R1, R3 |
| Astra 17: Dream all does not drain | D5 |
| Astra shutdown/authenticated lifecycle follow-up | R5 |
| Astra missing Dream graph outputs | D2 |
| Astra receipt/finalization window | R4 |
| Astra remote release-source scope | F1 |
| Astra rustdoc/AGENTS drift | F2 |
| Sonnet 2.1–2.4 overlapping product gaps | M1–M5, S2, D1, R5 |
| Sonnet 2.5 historical CSRF fixture annotation | W1, F2 |
| Sonnet 3.1 mutation/phase/audit split | D3 |
| Sonnet 3.2 exhausted budget discards pairs | R3, D4 |
| Sonnet 3.3 Rust-to-Rust upgrade path | R3 |
| Sonnet 3.4 predicate interpolation hardening | S3 |
| Sonnet event-order/replay follow-up | W4 |
| Owner: no FTS-only completion | S1–S3, F1–F2, R4 health integration |

Sonnet's additional correctness findings are retained; its claims that the console is functional and SIGTERM necessarily invokes the current handler are not accepted without evidence. Editing an unreleased initial SQL script is not itself released-data corruption; R3 nevertheless establishes a real upgrade path before new tables ship. Predicate interpolation is guarded today, so S3 is boundary hardening rather than an asserted exploit.

## Verification and completion rule

Each task includes executable regression code, exact file ownership, interfaces and focused commands. Small helper tests establish a first RED cycle; they do not replace the task's persisted/process/browser assertions.

Final completion requires:

1. All 17 Astra findings and reconciled Sonnet additions closed with the mapped behavioral checks.
2. Real configured Dream providers and enabled-workflow behavior, including more than one batch and crash-resumable pair budgets.
3. Learned memory **and actual semantic RAG** working through the installed daemon, with pinned real tokenizer/model, generation invalidation, background execution/reconciliation, isolation, provenance and relevance results. A synthetic harness or a degraded FTS query is not this evidence.
4. Browser and MCP/CLI flows tested against the same installed artifact; no in-process-only substitution.
5. Migration/recovery/update/rollback and shutdown tested on disposable roots, including failure injection and required-semantic readiness.
6. F2's Rust/frontend checks and explicit real-semantic/installed-artifact suites passing; docs state actual limits. No publication or live-data migration is implied.

## Plan self-review

Coverage: every confirmed finding maps above; the owner’s semantic requirement is mandatory in every subsystem plan. Self-review completed: local links resolve; all 39 frozen tool names have owners; planned file creation precedes dependent edits; all confirmed review rows and the owner’s semantic requirement have task coverage. Implementation sketches are the core changes, not claims that boilerplate or integration tests already exist. At execution, preserve each task's behavior-level acceptance assertions rather than implementing only its helper snippet.

