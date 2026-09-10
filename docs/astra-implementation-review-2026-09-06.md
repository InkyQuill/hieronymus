# Rust remediation implementation review — 2026-09-06

## Verdict and grades

The agents implemented substantial working software. This is no longer the largely disconnected port reviewed at `fcab0b5`. However, “fully implemented, reviewed, and no important findings” overstates the merged result. The happy paths are well tested; failure recovery, cross-plan integration, and completion claims are weaker. **Do not proceed directly to release acceptance.** Fix the data-loss and ownership failures first, then finish semantic reliability.

Reviewed tree: `02ffe1f` on `feat/rust-rewrite-proposals`, compared with the original `fcab0b5` baseline and the six remediation plans. The delta is 138 files, 40,150 insertions and 2,949 deletions, including tests. Size is not a quality measure.

Grades assess these particular delivered workstreams, not general model capability. Scale: 9–10 = complete with strong failure-path evidence; 7–8 = substantial delivery with important gaps; 5–6 = working foundation but material requirements incomplete; below 5 = substantially unusable. Attribution separates newly introduced defects, old gaps left open, and interactions introduced when plans merged.

| Workstream / implementer | Grade | Assessment |
| --- | --- | --- |
| R1–R5 / Claude Sonnet 5 | **8/10, B+** | Strong ownership, classification, migration and rollback work. R5 retains completed worker handles indefinitely. The later S2 integration breaks failed-start ownership cleanup; that is principally an integration regression, not evidence the original R2 lock was fake. |
| M1–M5 / GLM-5.3 Flash | **6.5/10, C+** | Real dispatcher, contracts and lifecycle work; useful transport tests. M5 adds a data-loss path in export and retains a second, weaker discovery client. The accepted silent-lexical deviation was not closed by the merged S work. |
| W1–W4 / Claude Sonnet 5 | **7.5/10, B** | Bootstrap, views, actions and event handling are real. Concept merging still has partial-commit/audit gaps; proposal materialization omits variants. The agent's residue note is partly stale: add_memory and concept deletion are now atomic in the inspected tree. |
| D1–D5 / GLM-5.3 Flash | **7/10, B−** | Real configured providers, normalized output, transaction tests and durable pair progress are strong improvements. Urgent failures retry without backoff; drain progress omits deterministic-only work; pair creation is not resource-bounded. |
| S1–S3 / GLM-5.3 Flash | **5.5/10, C−** | Tokenization and the demonstrated real-model recall path work. S2 readiness/recovery/configuration and strict RAG integration remain materially incomplete. The highest risk is trusting a readiness enum that does not establish service readiness. |
| F1–F2 | **Not graded / not implemented** | Correctly excluded from agents' completion claims. Still required after the repairs below. |

Overall engineering assessment: **about 7/10**. Retain the implementation and repair it; another rewrite is not justified. Passing review loops did catch defects, but did not establish whole-product completeness.

## Authority and scope

The uncommitted ADR 0016 is marked accepted on 2026-09-06. It supersedes ADR 0005 and the human-only portions of ADR 0011. The agents are graded against their original assigned plans; they are not penalized retroactively for implementing the previous lifecycle policy. **New follow-up work must follow ADR 0016**, retaining deterministic structured authority, revision checks and audit while designing autonomous decisions and immediate correction effects. Simply deleting the old guards would not implement the new policy.

All pre-existing dirty documentation, ADR 0016, `.claude/`, the original reports and original plans were preserved. No production implementation fixes, commits, merges, publication, user-service changes or live-data migration were performed in this review.

The owner's requirement remains: **both memory and semantic RAG must work. FTS-only is not a completion/release alternative.** Temporary degraded service must report its limits truthfully.

## Verification performed

- `cargo fmt --all -- --check`: pass.
- `cargo clippy --all-targets --all-features --locked -- -D warnings`: pass.
- `cargo test --all-targets --all-features --locked --no-fail-fast`: **912 passed, 0 failed, 2 ignored**, across 66 result summaries. The ignored cases are the real semantic qualification and the domain live ONNX test. Counts differ from the reports because this is the merged all-feature/all-target tree.
- `cargo doc --no-deps --all-features --locked`: succeeds with **16 warnings** (7 domain, 9 binary library), including the original two and new unresolved/private links. This is not a warning-clean documentation gate.
- Frontend `bun run typecheck` and `bun run test`: pass, **64 tests / 7 files**.
- Frozen `compatibility/snapshots` and `compatibility/fixtures`: unchanged from `fcab0b5`.
- Real `semantic_real` suite rerun: **1 passed**, 15.69 seconds. The observed seven-query English results match the recorded corpus. This exercises actual ONNX inference and authenticated MCP calls, with the healthy daemon hosted in the test process; it is not an installed-release or real-browser rehearsal.
- Five temporary Rust probes reproduced export overwrite, failed-start worker leakage, false semantic readiness, retained finished workers, and non-atomic concept merging. A separate real daemon/CLI probe reproduced enable reporting armed while the existing daemon remained failed. These probes assert the defective observations; their passing status means the bugs reproduced, not that the implementation is correct. The temporary test source was removed after running; retained local evidence is listed below.

The approved model acquisition command succeeded. ONNX runtime acquisition downloaded/extracted the pinned runtime but failed its final tree validation with `ONNX Runtime extraction does not match approved archive`. For the real-model rerun I independently verified the extracted library's SHA-256 against the recorded qualified value `1461ef7cc3d9e49982591721683cc3e3a55580aeca9a5254e7aac47b75ee4bab`, copied that exact library to a disposable path, and used the checksum-verified model and pinned tokenizer. **This does not establish that the normal acquisition workflow passes.** Track that separate release prerequisite in follow-up P3.

No fresh browser interaction or installed-artifact rehearsal was performed. Those remain F2 work. No language-wide relevance claim follows from seven English queries.

## High-priority findings

### A1 — Export can overwrite the authoritative database (new M5 defect)

Evidence: `crates/hiero/src/export.rs:62–99`. Opening SQLite read-only does not protect its path from the later `std::fs::write(output, text)`. Neither the CLI nor export rejects the database path, an alias to it, or other runtime files.

**Reproduced:** create a disposable database, then call `export::run(&config, &config.database_path())`. It returns success and replaces the SQLite header with JSON. This contradicts the implementation's “never copies or touches the live SQLite file” claim and can destroy the user's memory. Protect authoritative/runtime paths and aliases before writing; publish to a safe new destination atomically. Also use one read transaction: the current per-table reads can span different database snapshots during concurrent daemon writes. This is a read-only connection, not a coherent export snapshot.

Fix: C1. Rust error-handling/testing principle: external path input must be validated against the actual invariant, and error-path tests must prove state remains intact.

### A2 — Failed daemon startup releases ownership while a semantic worker survives (new S2/R5 integration defect)

Evidence: `crates/hiero/src/daemon/mod.rs:259–288`. The semantic worker starts before `TcpListener::bind`; token/discovery and subsequent controller construction are also fallible. WorkerGroup has no cleanup-on-drop guard. An early `?` drops JoinHandles, detaching the worker, and releases RootOwnership.

**Reproduced:** occupy the requested port, install a counted test arm, call Daemon::start, observe the bind error, reacquire RootOwnership, then observe the first daemon's semantic arm executing. The public daemon constructor can therefore leave a writer alive without root ownership. A command-line process normally exits after the error, but the library/in-process case is broken and the error path violates the central ownership invariant.

Fix: C2. Delay worker admission until prerequisites succeed and keep an RAII startup guard that cancels and joins any admitted workers before releasing ownership on every failure.

### A3 — Semantic “Ready” does not prove a query lane or usable generation (new S2 defect)

Evidence: `crates/hiero/src/daemon/semantic_worker.rs:388–400,447–463,516–531`. Query-lane `arm` errors are discarded, yet completion sets Ready. Cancellation also sets Ready without verifying an older generation exists. Startup accepts any active manifest without checking identity, index integrity or corpus coverage.

**Reproduced:** an arm succeeds for the indexing provider and fails for the query provider; the controller reports Ready with **zero query-lane installations**. R4's update gate now correctly consumes semantic state, but a false Ready defeats that gate.

Fix: C3. Derive readiness from verified query-lane installation plus a valid current generation or a genuinely empty corpus. Failed/cancelled work must not fabricate readiness. Do not swallow store, sample, or lane-install errors.

### A4 — Import-to-index recovery does not close the commit/enqueue window (remaining S2 gap)

Evidence: `application/memory.rs:719–734` commits import before queueing and converts enqueue errors into a field on a successful result. `daemon/semantic_worker.rs:516–531` queues missing work only when no active generation exists; ordinary polling reconciles job records rather than comparing the authoritative corpus with the active generation.

A crash after import commits but before enqueue, or an enqueue failure, leaves new documents unindexed when an older generation is active. Restart sees “some active generation” and marks Ready. “Missed wakeup is recovered” is true for an already-persisted job, not for missing durable work intent.

Additional defect: `semantic_recall.rs:347–365` deduplicates pending jobs solely by chunk count. Equal-count document replacement or a changed embedding identity does not mean the queued generation covers the same corpus. Source inspection; not separately crash-injected during this review.

Fix: C4. Persist indexing intent/revision in the authoritative import transaction, reconcile coverage on startup, and compare corpus revision plus identity rather than row count.

### A5 — Public RAG search is still lexical-only; cold failed semantics can be silent (remaining M2/S2 integration gap)

Evidence: `application/memory.rs:764–784` calls RagStore::search directly. It neither consumes the semantic lane nor requires readiness. `recall.rs:389` retains an empty-warning path when no lane is installed; the semantic controller does not propagate its Failed state into that response.

The successful S3 corpus uses `hieronymus_recall`; it does not establish semantic behavior of `hieronymus_rag_search`. A cold missing/unconfigured runtime can therefore coexist with ordinary successful lexical tool results. M's report explicitly accepted silence as a temporary deviation; the merged S work did not close it.

Fix: C5. Route strict RAG through the semantic/hybrid service with readiness enforcement and preserve the public result contract through a versioned change where necessary. Mixed recall may retain useful memory, but must report incomplete semantics.

### A6 — Configuring semantics after daemon startup does not heal that daemon (new S2 integration defect)

Evidence: `daemon/semantic_worker.rs:195–214` resolves UnconfiguredArm/OnnxArm once. Retrying the same UnconfiguredArm cannot see a newly written runtime path. `main.rs:693–703` persists configuration and probes a separate CLI process.

**Reproduced with real assets:** start an unconfigured daemon; run `hiero semantic enable --runtime … --json`; CLI exits 0 and reports `lane: armed`, `runtime_verified: true`; after six seconds the existing daemon still reports `failed: onnx runtime library is not configured`. The normal command recommends exactly the operation that fails to repair the running service.

Fix: C6. Make semantic configuration a daemon-owned mutation with explicit reload/arming acknowledgement, or an explicit supervised restart workflow. Use typed load errors, not `Option` that collapses malformed config into absence. Do not report the CLI's independent load as readiness of the running application.

### A7 — Concept merge is neither all-or-nothing nor atomic with audit (remaining W3 gap)

Evidence: `application/admin/actions.rs:554–625`. Each source merges in its own transaction; audit is appended afterward. Preflight does not prevent a later database failure or concurrent change. Errors inside the source loop also bypass the later “these merges committed” diagnostic.

**Reproduced:** a trigger rejects audit_log insertion; run_action returns an error, but the source concept has already become merged. The explicit audit-failure message is honest about that one branch; it does not repair the missing audit or make the multi-source operation atomic. Under ADR 0016 this remains relevant to reliable maintenance, although manual merging is no longer a required ordinary workflow.

Fix: C7. Extract transaction-taking merge primitives and commit every selected merge, provenance and audit together; test a failure on the second source as well as audit failure.

### A8 — Urgent Dream provider failures can retry at every two-second gate (new D5 defect)

Evidence: `daemon/dream_worker.rs:481–527`. Persistent urgent backlog submits another attempt independently of the interval. There is no failure backoff/circuit state; a failed run leaves the backlog urgent.

A fast provider/config failure can repeatedly create failed runs and, for reachable paid providers returning failures, repeated network requests. The agent disclosed this accurately, but “most consequential deferred item” should have been promoted above polish. ADR 0016 also requires bounded retryable consolidation during outages.

Fix: C8. Persist or reconstruct retry eligibility, use bounded exponential backoff/jitter, reset on relevant config change/success, and preserve durable observations. Manual retry must be explicit and still coalesced.

## Medium-priority findings and incomplete acceptance

### A9 — Completed connection workers accumulate for the daemon lifetime (new R5 defect)

`daemon/workers.rs:65–82` appends every JoinHandle and only drains at shutdown. **Reproduced:** 100 completed jobs leave live_count=100. The server uses this group for accepted connections, so retained handles grow with total historical requests. Reap finished workers during normal operation, retain panic diagnostics, and distinguish live work from retained handles (C2).

### A10 — Dream drain progress omits deterministic-only work (remaining D5 gap)

`dreaming.rs:686–710` can select another cycle because deterministic work exists, but progress is solely `record.input_count`; zero crystallization input stops drain_batches. The final blocked guard at 734–750 only checks pending short-term input. Queued link pairs can remain while a manual drain reports completion. The per-pair durability is good: this is incomplete draining, not the original permanent-loss bug. Define progress/pending counts for each eligible work class and keep per-call resource limits explicit (C8).

### A11 — Pair snapshot materialization is unbounded by the processing budget (new D4 scalability issue)

`dream_link_progress.rs:269–309` materializes canonical_pairs for the full session before applying the pair budget. The helper allocates n(n−1)/2 pairs. A 10,000-distinct-crystal session entails 49,995,000 pairs, regardless of budget=1. This is a direct complexity observation, not a measured production slowdown. Use a persisted bounded pair cursor or bounded snapshot slices without losing pair coverage (C8).

### A12 — M5 still uses a second discovery client without the R5 instance probe (remaining M/R integration gap)

`daemon_client.rs:92–106` reads discovery/token and returns a client without authenticated instance comparison; `main.rs:1047` still selects it. R5's lifecycle client does the stronger check. A validly shaped stale record is not treated as stale by this adapter, and opt-in autostart only handles missing discovery. Consolidate the clients and exercise stale/reused-port/instance mismatch via the actual CLI and stdio surfaces (C9).

### A13 — Proposal materialization omits approved/forbidden variants

`application/admin/actions.rs:940–1013` materializes the canonical rendering but omits approved/forbidden variant facets present in the stored proposal. Preserve these evidence/variant semantics through the domain boundary (C7). The earlier M report’s strict-only deferral is not by itself a current missing-source finding: D now writes its normalized concept proposals into strict_concept_proposals (`dreaming.rs:1419`, `concepts.rs:1950`), which the current listing reads. Do not build an unnecessary second proposal store merely from that stale handoff note. Under ADR 0016 this is optional inspection and autonomous-decision evidence, not a human approval inbox to complete.

### A14 — English relevance smoke test is real but not full literary-language qualification

The real seven-query S3 test is useful and reproducible. It intentionally uses English queries/documents and lexical short-term memory. S3's plan also required the project's actual source/target examples before release; that step is not done. Do not confuse an English-first compact-memory policy with semantic competence over Japanese/Russian source passages. Validate declared supported languages and story contexts; choose/requalify a model only if measured requirements require it. Do not weaken expected results to keep the existing model (P2).

### A15 — Documentation and release prerequisites still need work

There are 16 rustdoc warnings, obsolete Python-era usage, and the runtime acquisition failure observed above. F1/F2 are unimplemented, so packaging, remote release resolution, host installation and installed/browser rehearsal remain unverified. The original machine qualification record's closed schema need not automatically be expanded: first determine whether F's actual consumer needs an additional record. A linked reproducible semantic report may satisfy the evidence boundary without inventing new gate infrastructure (P3).

## Task-by-task assessment

“Substantial” means real implementation with the listed unresolved acceptance issue; it is not a completion sign-off.

| Task | What is actually present | Remaining assessment |
| --- | --- | --- |
| R1 | Bounded startup classifier; config reads no longer persist legacy conversion | Delivered for original config set; new semantic.conf needs typed validation in C6 |
| R2 | Shared OS ownership guard; daemon/upgrade/recovery contention tests | Delivered; later failed-start worker integration violates lifetime invariant (A2) |
| R3 | Ordered v1→v2 SQL, markers, backups/journal target handling; merged M3 columns | Delivered; avoid future duplicate code-side schema ownership |
| R4 | Unknown doctor codes rejected; ordered rollback; receipt finalization; semantic status gate | Delivered mechanically; false S2 readiness weakens integration (A3) |
| R5 | Stable token, SIGTERM, lifecycle aliases, joined workers and idle socket wakeup | Substantial; reap handles and unify M client (A9/A12) |
| M1 | Real Application dispatcher, seven series/session tools and envelope mapping | Delivered |
| M2 | Eight memory/RAG operations and independent deterministic_contract | Substantial; strict RAG and missing-lane reporting remain (A5) |
| M3 | Transactional structured approval/archive/replacement, revisions and idempotency | Delivered against original policy; ADR 0016 requires a new authority design |
| M4 | Sixteen graph adapters, facet updates and proposal listing | Delivered; D now writes proposals into the listed store; variant materialization is a W3 issue (A13) |
| M5 | HTTP/stdio matrix, daemon feedback, generic tool CLI, export and plugin generator | Substantial; export data loss and split client (A1/A12); generated human-only advice is now obsolete |
| W1 | CLI grant bootstrap, synchronous fragment scrub, session exchange, Origin correction | Delivered; browser rehearsal remains F2 |
| W2 | Ten real admin projections with filters/paging | Delivered at projection level; variant materialization is a W3 issue |
| W3 | Thirteen actions, real provider checks, accessible input dialogs | Substantial; concept merge and variants (A7/A13) |
| W4 | Cursor resumption, dedupe, cleanup, stale-response guards/coalescing | Delivered; no new high-confidence correctness defect found in this pass |
| D1 | Enabled-workflow resolver and actual per-profile/model calls | Delivered |
| D2 | Normalized graph output and context/target validation | Delivered against original lifecycle policy; autonomous rule decisions are new design work |
| D3 | Transactional domain/phase/audit for deterministic and provider work | Delivered; per-pair audit granularity for D4 is a justified adaptation |
| D4 | Durable pair ledger, resume, per-pair transaction and audited tombstones | Substantial; pair creation unbounded (A11) |
| D5 | Shared real provider controller, scheduled/manual routes, batch draining and shutdown | Substantial; retry/backlog and deterministic drain gaps (A8/A10) |
| S1 | Pinned real WordPiece tokenizer and changed identity | Delivered; do not confuse identity rejection with automatic rebuild recovery |
| S2 | Supervised worker, real rebuild execution, lane installation and status | Partial acceptance: A2–A6 are material correctness/integration issues |
| S3 | Real English corpus, isolation/provenance checks and predicate builder | Substantial; auto-recovery failures not exercised, actual language acceptance missing |
| F1/F2 | Not part of these implementations | Remain future work, with prerequisites P2/P3 |

## Evaluation of the agents' rulings

Good adaptations: matching real session/result keys; aligning R3's schema with the audited consumer; sharing one event hub; using a real loopback provider instead of a mock interface; changing D4 to per-pair atomicity with durable audit; leaving frozen historical fixtures unchanged. These preserve intent and have concrete evidence.

Weak rulings: treating runtime warning requirements as satisfied by a README/fixture; leaving two clients after R5 merged; accepting code-side schema ensures as indefinite ownership; classifying persistent urgent retry and missing merge atomicity as harmless minors. External dependencies justify temporary seams, not final omissions after those dependencies land.

The S3 qualification-record deviation is reasonable to disclose, but a new schema is not automatically required. The real issue is whether the eventual release evidence accurately proves required behavior. Conversely, “all curated sources top-3” is correctly supported within this small English corpus; it should be credited, not dismissed because other paths are incomplete.

No inference about deliberate misrepresentation is warranted. The recurring process problem is testing the chosen implementation locally while judging completion against a broader plan that was not fully replayed after integration.

## Follow-up order and evidence

Start with [merged-port correctness](superpowers/plans/2026-09-06-merged-port-correctness.md): A1/A2 first, then semantic truth/recovery, remaining transactions, Dream bounds and client integration. Continue with [product and release readiness](superpowers/plans/2026-09-06-product-release-readiness.md): ADR 0016 design, realistic language/host acceptance, then the existing F1/F2 plan with corrected prerequisites. No FTS-only alternative.

Local review evidence (ephemeral, not release artifacts): `/tmp/astra-review-clippy.log`, `/tmp/astra-review-tests.log`, `/tmp/astra-review-doc.log`, `/tmp/astra-review-frontend.log`, `/tmp/astra-review-probes.log`, `/tmp/astra-review-export-probe.log`, `/tmp/astra-audit-probe.rs`, `/tmp/astra-live-semantic-probe.py`, `/tmp/astra-live-semantic-probe.log`, `/tmp/astra-review-semantic-real.log`, `/tmp/astra-review-acquire-runtime.log`. Reproduction instructions and expected corrected behavior are carried into the follow-up plans so these temporary logs are not the only actionable record.
