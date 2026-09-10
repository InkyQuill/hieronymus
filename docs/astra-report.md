# Independent Rust implementation review

Date: 2026-09-05. Reviewed commit: `fcab0b5e4dea1e13a3b68884d441fb894c0e0417` (`feat/rust-rewrite-proposals`). The working tree was clean at the start. This review changes documentation only.

Updated the same day with an explicit cross-check of all 15 ADRs, their supersession notices, and the 2026-09-03 owner amendments. Findings 13–17 and the ADR assessment below are additions from that pass.

## Verdict

The port contains substantial, tested domain and infrastructure work, but it is **not a complete application and is not ready for cutover**. The supplied report correctly discloses the missing MCP dispatch. It understates the remaining work: browser authentication is unfinished, most admin operations are absent, real dreaming is disconnected, disabled workflows still execute, semantic tokenization is unsuitable for the model, and daemon ownership and updater recovery have correctness gaps.

I recommend keeping the implementation and completing focused integration and correctness slices. A second wholesale rewrite would discard useful work without addressing the main failure: qualification scaffolding and isolated module tests have been treated as evidence of complete user workflows.

**Owner scope decision (2026-09-05):** this app must deliver both working memory and semantic RAG. ADR 0013 permits an FTS-only baseline in general, but the owner explicitly rejected that alternative for this remediation. Findings 7–8 are release blockers alongside the agent/console, rule-contract, and ownership/recovery defects. FTS remains part of hybrid retrieval; it cannot substitute for the semantic lane at completion.

## ADR authority and assessment

[ADR 0008](adr/0008-rust-reimplementation-authority-and-cutover.md) makes accepted ADRs authoritative over specifications/fixtures, tests, Python behavior, and proposal documents. Narrower later ADRs control explicit conflicts. An implementation-controller note in the SDD ledger does not amend an accepted ADR. A missing item in the handoff queue does not remove a product requirement.

The 2026-09-03 amendment to ADR 0008 also matters: cutover review is an **owner-reviewed list of questions**, not an attestation system or a request to build more gate tooling. The acceptance tests recommended here supply useful verification; no additional certification machinery is proposed.

| ADR | Applicable decision and assessment |
| --- | --- |
| [0001](adr/0001-dream-conf-plaintext-secrets.md) | Plaintext local configuration and redacted surfaces remain current; provider ownership moved to 0007. Keeping plaintext secrets is intentional, not a defect. The port has explicit secret/projection types, but this is not a complete redaction certification. |
| [0002](adr/0002-ink-react-tui-migration.md) | Superseded by 0014. No Ink, React TUI, Node runtime, or Python wheel requirement should be restored. |
| [0003](adr/0003-multilingual-concept-centered-memory-graph.md) | Historical model/provenance reference, with schema ownership transferred to 0010/0011. The rule-crystal-only authority is superseded. Do not use it to reject structured term tables. |
| [0004](adr/0004-replace-ink-tui-with-opentui.md) | Superseded by 0014. Missing OpenTUI is not a port gap. |
| [0005](adr/0005-product-vision.md) | Product boundaries, primitive tools, English-first memory, bounded multi-phase dreaming, and drain behavior remain applicable. Backend authority, terminal UI/runtime requirements, and autonomous rule activation are superseded. Findings 1, 5–6 and 17 identify remaining gaps; unsupported dream output sections are unfinished product scope. |
| [0006](adr/0006-alpha-versioning-and-release-authority.md) | Workspace version 0.7.0 and the daemon's alpha display align with the pre-1.0 decision. No major release approval was inferred or exercised. |
| [0007](adr/0007-provider-catalog-and-workflow-assignments.md) | Separate provider catalog is implemented. Runtime per-workflow provider/model selection and disabled-state behavior are not: findings 5–6. The ADR already chooses multiple providers/models; this is not a pending design ruling. Runtime migration writes also violate the later upgrade boundary: finding 16. |
| [0008](adr/0008-rust-reimplementation-authority-and-cutover.md) | One Rust binary/workspace is implemented; successful tests do not constitute owner cutover approval. Missing behavior cannot be excused by Python compatibility or the handoff queue. Apply the certification-light amendment without weakening behavioral requirements. |
| [0009](adr/0009-runtime-topology-and-daemon-lifecycle.md) | Daemon and database-free stdio adapter roles exist. Daemon workers, exclusive ownership, authenticated instance discovery, bounded shared startup classification, and daemon-routed normal mutations are incomplete: findings 5, 8–9, 14–15 and follow-ups below. Initial macOS support is superseded by 0013. |
| [0010](adr/0010-data-locations-schema-ownership-and-upgrade.md) | Existing root/filename, explicit converter, backup, and journal mechanisms are substantial partial compliance. Runtime config migration remains outside the exclusive protocol (16); recovery/receipt and ownership findings remain relevant. Full production-shaped release recovery was not re-rehearsed in this review. |
| [0011](adr/0011-deterministic-terminology-and-graded-memory.md) | Structured term authority and advisory conflict markers exist, but the required separate deterministic contract is absent from recall responses (13). Separate termbase tools do not satisfy this requirement. End-to-end authenticated rule approval remains part of missing transport wiring. |
| [0012](adr/0012-mcp-transport-authentication-and-discovery.md) | Apply the owner amendment: session-cookie WebSocket auth, no separate CSRF token, no credentials_rotated ceremony. Loopback binding and native bearer checks exist. Console bootstrap and persistent token behavior remain deviations (2, 11). Requiring Origin on all reads is an implementation overrestriction, not an ADR obligation (3). |
| [0013](adr/0013-semantic-index-and-platform-support.md) | Linux x86_64 only is correct. The ADR permits FTS-only, but the owner explicitly excluded that option for this remediation. Derived storage, identity and isolation mechanisms exist. Real semantic service execution/tokenization are incomplete (7–8); a qualified synthetic harness does not establish a working language retrieval pipeline. |
| [0014](adr/0014-web-console-replaces-terminal-ui.md) | Svelte asset embedding/build separation aligns. Required config/admin launch and browser contracts do not (2–4). Headless operations must remain usable without a browser. |
| [0015](adr/0015-mcp-protocol-and-transport.md) | The code pins 2026-07-28, serves /mcp, uses newline JSON-RPC stdio, and removes the private operation bridge as directed. Registry semantic parity is still missing (1). This review checks those local decisions and existing tests, not every clause of the external protocol specification. |

**Corrections to the supplied report's rulings:** the omitted recall-contract section is a violation of ADR 0011, not a non-gap; multi-provider dispatch is already selected by ADR 0007; waived CSRF and removal of the private MCP bridge are valid; Linux-only support is valid; semantic enablement remains optional under ADR 0013. Do not restore older decisions that the accepted amendments explicitly supersede.

## Verification and limits

Executed against this checkout:

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | Passed |
| `cargo test --all-features --locked` | **583 passed, 0 failed, 1 ignored**, including doc-test suites |
| `cargo doc --no-deps --all-features --locked` | Passed with two rustdoc warnings |
| `bun run --cwd frontend typecheck` | Passed |
| `bun run --cwd frontend test` | 16 passed across 4 files |
| Frozen inputs | No diff in `compatibility/fixtures` or `compatibility/snapshots` between `8a8d614` and reviewed HEAD |
| Temporary-root process/HTTP probes | Confirmed missing console commands, missing-Origin rejection, unsupported admin surfaces, duplicate daemon ownership, credential replacement |
| Temporary release/update probe | Candidate doctor exit 42 incorrectly accepted as healthy |

The supplied count of 579 is not the result of this all-features run; the difference alone is not a defect. Green tests are real, but do not establish complete application functionality.

The probes used the built Rust binary, disposable data/app directories, and ephemeral loopback ports. No real book databases, installed services, or agent configuration were changed. The updater probe used a synthetic executable and a custom unit directory, so it never contacted the real systemd manager. All review-started daemons were stopped.

This was a targeted whole-application review, not an exhaustive line-by-line audit. I did not execute the ignored live semantic test, benchmark retrieval relevance, perform a real managed-service update, rebuild the release archive, or run a real browser session. Browser conclusions below combine source inspection, real HTTP responses, and browser documentation. Python baseline checks were not rerun; this report does not claim new Python implementation verification.

The ADR follow-up added two temporary-root process probes: a malformed `dream.conf`, and a SQLite database containing only the supported schema-version marker. Both incorrectly started and published discovery. Earlier suite results above belong to the initial review; suites were not rerun for this documentation-only follow-up. The concurrently present `docs/sonnet-report.md` was left untouched.

## Findings

Severity: **High / P1** blocks the affected production workflow or safe cutover. **Medium / P2** is a concrete follow-up. Findings marked “source” were established by call-path inspection rather than an independent failure-injection run.

### 1. High: 38 advertised MCP tools still have no implementation dispatch

Evidence: `crates/hiero/src/daemon/registry.rs:107` (`McpRegistry::call`) only handles `hieronymus_status`; every other tool returns `NotPorted`. Arguments are unused and the registry has no domain-service context. The existing `unported_tools_report_not_ported` test explicitly expects this state.

This confirms the disclosed gap. Listing 39 schemas does not implement remember, recall, terminology, sessions, concepts, or RAG operations. A status-only call is insufficient as a cutover smoke test.

**Change:** inject application services into dispatch and implement the frozen operations without inventing new response shapes. Track every advertised tool through argument validation, domain invocation, result/error serialization, and persisted effects. Exercise a real remember → session completion → recall workflow over both HTTP and stdio, plus strict-term conflicts and unknown IDs.

### 2. High: there is no usable console authentication bootstrap

Evidence: `crates/hiero/src/main.rs:22` lists the supported commands and the command dispatcher lacks `admin` and `config`. Both commands returned exit 2, `unknown command`, in the process probe. `frontend/src/web/lib/api.ts:14` immediately makes API requests; searching the frontend found no launch-grant exchange implementation. The exchange endpoint exists at `crates/hiero/src/daemon/rest/mod.rs:179`, but the application has no client flow that uses it.

Embedding the existing frontend only packages this missing integration. A fresh browser cannot obtain the session required by the console API.

**Change:** implement the CLI-to-browser bootstrap specified by ADRs 0012 and 0014, including grant delivery, one-time exchange, session establishment before data loading, expiry/restart recovery, and credential redaction. ADR 0012 explicitly forbids URL query strings; avoiding fragments as well was a review recommendation, not wording found in that ADR. The remediation plan chooses a short-lived fragment grant with immediate history removal and no URL logging; installation bearer/session credentials never use this channel. Test from a fresh browser context using the actual embedded bundle.

### 3. High: authenticated browser GET requests are rejected

Evidence: `crates/hiero/src/daemon/rest/mod.rs:127` requires an exact Origin; `guard_api` applies that condition to reads as well as mutations. After manually exchanging a grant, a real `GET /api/admin/dashboard` with the valid session cookie but no Origin returned `403 {"error":"forbidden_origin"}`.

The frontend uses ordinary same-origin `fetch` GET requests. Browsers generally omit Origin on same-origin GET/HEAD, and Origin is a forbidden script-controlled header. See [MDN's Origin documentation](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Origin). Consequently, fixing grant exchange alone will still leave dashboard/settings loading broken.

**Change:** distinguish safe reads from mutations. Keep session and Host checks, reject explicitly foreign origins, and use a browser-compatible policy for absent Origin on safe reads. Keep strict cross-site protections for mutations and WebSocket upgrades. Do not fix this by trying to set Origin from JavaScript. Add a browser-backed test that does not manufacture headers absent from normal browser traffic.

### 4. High: most advertised admin views and actions are absent

Evidence: `crates/hiero/src/daemon/rest/admin.rs:29` supports only Crystals and Lessons, while the dashboard advertises ten views. At line 245 the general action dispatcher implements only `reinforce_crystal`; manual dreaming has a separate route. Other advertised actions are unresolved.

With valid session and Origin, the process probe returned:

```text
GET /api/admin/snapshot?view=concepts -> 400 unsupported admin view: Concepts
POST /api/admin/actions/add_memory   -> 404 unknown_admin_action
```

**Change:** complete the view and action adapters over the existing stores: concepts/renderings, short-term memory/sessions, dream runs/proposals/audits, and the advertised create/edit/delete/merge/split/review operations. Until then, capability metadata should not imply that all advertised operations work. Add parameterized coverage of every advertised view and command, including persisted mutation results.

### 5. High: production dreaming is still attached to the deterministic provider

Evidence: `crates/hiero/src/daemon/rest/admin.rs:277` always constructs `DreamService::open(&config, DeterministicDreamProvider)`. The workflow gate at `crates/hieronymus/src/dreaming.rs:2007` correctly rejects this provider when an enabled workflow requires a configured LLM. `LlmDreamProvider` exists in `dream_providers.rs:280`, but the daemon never constructs it for a dream run.

Thus real provider configuration and successful connectivity checks do not make manual dreaming usable. This is separate from the reported multi-provider limitation: **even one configured LLM provider cannot run through this admin path**. Opening the service can fail before any run row exists; the WebSocket failure event is then the only immediate run feedback.

There is also no production dream scheduler in the daemon startup/runtime; schedule settings are exposed but no scheduled worker consumes them.

**Change:** resolve provider/model assignments in the production orchestration layer and use the real transport. Dispatch per workflow is required by ADR 0007; weakening the fail-closed gate to force all workflows onto one provider/model would not comply. Add a daemon-owned scheduler with explicit shutdown behavior and an end-to-end manual run against a local mock provider that proves the intended HTTP request occurred. Deterministic execution should require an explicit deterministic assignment/test seam.

### 6. High: disabled workflows are validated as disabled but executed anyway

Evidence: `validate_workflow_wiring` skips disabled workflows at `crates/hieronymus/src/dreaming.rs:1975`. However, `execute_passes_inner` loops unconditionally over `DREAM_WORKFLOW_NAMES` at line 557 and calls the provider at line 582. There is no enabled check in that execution loop. `start_phase_run` records the injected provider, rather than resolving the individual disabled workflow's assignment.

With pending input, a disabled pass still runs and can produce mutations; with an injected LLM, it also incurs an unwanted provider call. Startup validation and execution therefore enforce different policies. This is a source-confirmed bug, not merely the multi-provider design question.

**Change:** derive one validated execution plan and execute only its enabled passes. Define how coverage validation behaves if coverage_audit is disabled; do not silently archive unvalidated input. Add a recording provider test asserting zero calls for disabled phases and no disabled-phase mutations. Global scheduling enablement and an explicit manual override should be treated separately from individual workflow enablement.

### 7. High: semantic text preprocessing feeds invented vocabulary IDs to MiniLM

Evidence: `crates/hieronymus/src/semantic_recall.rs:98` maps each UTF-8 byte to `((byte * 31 + position) % 30000) + 1`. The `load_lane` function in `semantic_arming.rs` attaches this ByteFoldTokenizer to the real ONNX provider. `semantic_embeddings.rs:24` correctly identifies the model vocabulary as WordPiece, but does not actually tokenize text with it.

These IDs select unrelated learned vocabulary entries; matching document/query transformations only establishes consistency, not semantic meaning. The 512-position check also becomes a 512-byte limit for this path, disproportionately affecting non-ASCII text. Qualification on synthetic token sequences establishes runtime/index behavior, not language preprocessing correctness.

The [model publisher's usage instructions](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2) use the model's tokenizer, attention-aware pooling, normalization, and a defined truncation policy. Byte folding is not an alternative implementation of that tokenizer. No retrieval-quality benchmark was run here; the preprocessing mismatch is visible directly in the production call path.

**Rebuild this part before enabling semantics:** acquire and verify the matching tokenizer artifacts, implement the actual normalization/WordPiece/special-token pipeline, and define chunking/truncation in tokens. Include tokenizer revision/checksum and preprocessing policy in generation identity. Invalidate and rebuild every byte-fold generation. Add token-ID golden tests against the reference tokenizer and a small semantic relevance test using paraphrases and unrelated distractors. ADR 0005 chooses English-first prose with preserved non-English forms; evaluate that actual corpus. A mandatory multilingual model is not an ADR requirement.

### 8. High: durable semantic jobs have no production executor

Evidence: `crates/hieronymus/src/semantic_jobs.rs:670` implements `run_rebuild`, but all call sites are in `crates/hieronymus/tests/semantic_jobs_port.rs`. Neither `crates/hiero/src/main.rs` nor daemon startup calls it. `arm_recall_service` likewise has no production caller. The upgrade enqueues rebuild work, and recall repair can schedule work, but no production worker drains it.

This extends the disclosed missing recall wiring. Calling `arm_recall_service` alone will not build an active generation or complete queued repairs. The enable command's arming verdict constructs and drops a lane; it does not establish a daemon-owned running service.

**Change:** add a daemon-owned job executor with startup reconciliation, leases, cancellation, bounded resources, and shutdown joins. Wire model enablement, generation creation, rebuild execution, activation, and recall into one observable flow. Verify ingest → queued job → active generation → semantic recall → restart/repair through application entry points.

### 9. High: data-root ownership is not enforced by the daemon

Evidence: `Daemon::start` at `crates/hiero/src/daemon/mod.rs:151` checks database/journal state, then binds the requested port and overwrites discovery/token files. It holds no data-root ownership lock. `upgrade.rs:375` has a migration lock, but daemon startup does not participate in it; `daemon_start_blocker` at line 273 checks only the cutover journal.

The process probe started **two live daemon processes on the same root with different ephemeral ports**. Both succeeded. The second replaced the token/discovery; using the token now on disk against the first daemon returned 401. The first remained alive.

This also leaves a migration/start race: a read-only liveness precheck and later journal check do not mutually exclude a daemon from starting during pre-journal staging or recovery. Two independent daemons defeat the intended single-owner lifecycle and make update/migration ownership assumptions unreliable.

**Change:** use one shared OS-backed ownership protocol for daemon, migration, and recovery, acquired before opening/mutating authoritative state or publishing readiness. Hold daemon ownership until all database users have stopped. Test different-port duplicate starts and concurrent daemon-start versus migrate/recover. Preserve different roots as independent installations.

### 10. High: updater health acceptance and rollback can report success incorrectly

Evidence: `crates/hiero/src/update.rs:394` runs the candidate's doctor; the failure branch tests only `health_code == 2`. A synthetic candidate whose doctor exited **42** was accepted: updater exit 0, outcome `updated`, and `health check: healthy`. An `.output()?` spawn failure after switching links also bypasses the rollback branch.

The rollback implementation at `update.rs:474` restores links/unit text and calls service start, but never stops an already-started failed candidate or reloads the restored unit. It ignores restoration errors and subsequently deletes the candidate directory. The managed-service consequences are source findings, not a claim of a live systemd rehearsal: a running candidate may survive while links claim the previous version, and cached unit state may still target the candidate.

**Change:** accept only explicitly supported doctor outcomes, route every post-switch error through recovery, stop the candidate first, restore links/unit, reload the manager, and restart/verify the previous instance when appropriate. Report rollback failures honestly and preserve artifacts needed for recovery. Poll authenticated daemon readiness/version with a bounded deadline; a TCP connection or diagnostic warning alone is not proof the intended version started. Test unexpected exits, spawn errors, delayed startup, and failed rollback using a stateful manager seam, then rehearse in a disposable managed environment.

### 11. Medium: ordinary daemon restart silently rotates a per-installation token

Evidence: `crates/hiero/src/daemon/mod.rs:190` unconditionally generates and writes a bearer token on startup. ADR 0012's amendment calls for one static per-installation token; an explicit rotation may invalidate existing clients, but every restart should not implicitly be a rotation.

**Change:** load and validate an existing protected token, creating it only when absent. Give deliberate rotation its own path. Add restart identity and permission tests, plus adapter behavior after explicit rotation. This should be implemented under the ownership lock from finding 9.

### 12. Medium: fixture-specific success behavior is compiled into provider checks

Evidence: `crates/hiero/src/daemon/rest/providers.rs:60` bypasses the transport for any host ending in `.invalid`; `fixture_probe` returns success and `synthetic-model`. The comment explicitly says this exists to preserve oracle fixture behavior. It is not confined to tests.

An invalid placeholder endpoint should not produce a successful production connection check. Frozen fixtures describe a fake network environment; they do not require production code to recognize that environment by hostname.

**Change:** inject the fake provider transport from tests and use the real probe for every production profile. Add a negative production-path test for an invalid endpoint. Keep the frozen fixture bytes unchanged while fixing the harness setup.

### 13. High: recall omits the deterministic contract required by ADR 0011

Normative requirement: [ADR 0011, Decision](adr/0011-deterministic-terminology-and-graded-memory.md#decision) says, “Every recall response also includes a separate deterministic-contract section when applicable.” The accepted [terminology spec, Recall Result](superpowers/specs/2026-08-31-rust-terminology-memory-design.md#recall-result) explicitly names `deterministic_contract` alongside recall ID, ranked results, and warnings.

Evidence: `crates/hieronymus/src/recall.rs:130` defines only `recall_id`, `hits`, and `warnings`. Line 368 computes the contract, but line 438 returns no contract; it is used only to annotate advisory RAG hits. This affects the library independently of the absent MCP dispatch. Applicable rules can therefore be missing from a response even though the service already knows them.

The controller ruling at `.superpowers/sdd/handoff-rust-port/progress.md:167` cites Python's bare-list response and separate termbase tools to dismiss the requirement. That reverses ADR 0008's authority order. Neither an old response shape nor a reviewer/controller ledger note supersedes the accepted ADR.

**Change:** return the independently computed structured contract in the recall result and transport DTO, including when ranked hits are empty or limited. Reconcile affected compatibility expectations through the documented versioned change process with ADR 0011 as authority. Test that active rules remain visible independently of result ranking, FTS hits, semantic availability, and result limits. Removing this feature would require an explicit accepted amendment, not another controller ruling.

### 14. High: normal CLI feedback mutates SQLite outside the daemon

Normative requirement: [ADR 0009, Decision](adr/0009-runtime-topology-and-daemon-lifecycle.md#decision) says, “All domain mutations used by normal CLI, MCP, and frontend workflows go through the daemon.” Offline exceptions are exclusive maintenance and must refuse a daemon-owned root.

Evidence: `crates/hiero/src/main.rs:845` opens `FeedbackStore` directly and records the outcome in the CLI process. It neither proxies to the existing authenticated `POST /recall/feedback` route nor takes exclusive maintenance ownership. Feedback is an ordinary memory mutation, not database repair.

**Change:** make the CLI an authenticated client of the existing daemon feedback route, sharing its request/result mapping. It should not open the application database. Test it while the daemon runs, and confirm that a missing daemon produces the intended discovery/startup behavior rather than an alternate direct writer. This is a concrete architectural deviation even when SQLite happens to serialize the write successfully.

### 15. High: startup accepts invalid configuration and incomplete schema state

Normative requirement: [ADR 0009, Decision](adr/0009-runtime-topology-and-daemon-lifecycle.md#decision) requires a shared bounded classifier covering database/config state and journal state before binding or discovery. It explicitly rejects corrupt/partial state and legacy config, with actionable migration diagnostics. It does not require an expensive full integrity scan on startup.

Evidence: `Daemon::start` at `crates/hiero/src/daemon/mod.rs:151` checks the journal and database classifier, but not configuration. `crates/hieronymus/src/db.rs:154` accepts the metadata table's supported version without validating required application table presence. `open_migrated` then treats that database as ready.

Independent process probes confirmed both failures:

```text
dream.conf contains invalid TOML           -> daemon starts; daemon.json published
database has only hieronymus_meta(version=1) -> daemon starts; daemon.json published
```

**Change:** implement the bounded shared classifier prescribed by the ADR: required schema objects/markers, supported config formats, and journal state, with no conversion side effects. Reuse it in startup and maintenance preflight. Reject before binding/publishing; retain expensive integrity/conversion checks for explicit maintenance. Tests should assert both the error diagnostic and the absence of published readiness.

### 16. High: ordinary configuration loads still perform legacy migration writes

Normative requirements: ADRs [0009](adr/0009-runtime-topology-and-daemon-lifecycle.md) and [0010](adr/0010-data-locations-schema-ownership-and-upgrade.md) place legacy conversion behind explicit exclusive upgrade, backup, staging, and promotion. A normal read must not become an alternate migration protocol.

Evidence: `crates/hieronymus/src/provider_config.rs:124` calls legacy migration from `load_provider_catalog`; lines 185–189 save provider config and resave dream config while ignoring the latter error. `crates/hieronymus/src/dream_config.rs:165` detects legacy workflows and saves during load. These functions are used by normal REST settings/provider/admin paths and `DreamService::open`. The read-only resolver variants already exist but are not consistently used there.

Consequences include unbacked two-file conversion, a partially applied migration if the second write fails, and canonical reserialization outside the comment/ordering-preserving upgrade path. The startup omission in finding 15 makes these legacy states reachable by ordinary daemon requests.

**Change:** make runtime loads parse/validate-only and return explicit migration-required errors for legacy formats. Keep all conversion writes in the exclusive staged upgrade protocol, preserving comments, ordering, supported unknown keys, and credentials. Add byte-preservation tests for ordinary GET/load operations on legacy files, plus failure tests proving only explicit migrate can promote converted config.

### 17. High: Dream all processes one capped selection rather than draining

Normative requirement: [ADR 0005, Scheduling And Drain Behavior](adr/0005-product-vision.md#scheduling-and-drain-behavior) requires successive capped cycles until pending memory is drained once a manual/scheduled/urgent/CLI/MCP run starts. This product behavior is not superseded by the later Rust ADRs.

Evidence: `crates/hieronymus/src/dreaming.rs:418` implements `run_all` as one `run_locked` call; line 448 invokes one evidence-pass cycle. Lines 528–530 select at most `max_short_term_memories_per_run`; the SQL at line 743 enforces that limit. There is no outer drain loop in `run_all`, and the admin caller invokes it once. A backlog larger than the selected cap therefore remains after the reported completed run. This is a source-path finding, not an independently seeded backlog reproduction.

**Change:** add bounded successive-cycle orchestration with progress, cancellation, no-progress detection, and preserved per-cycle mutation/provider caps. Do not implement drain by removing the selection limit. Test a backlog larger than two cycles, failure after one committed cycle, and resumption without duplicate persistence.

## Additional scope and maintainability follow-ups

- **Dream output coverage remains partial.** `dreaming.rs:319` explicitly ignores provider `concepts`, `facets`, `supersede`, and `reinforce` sections with audited warnings; concept-name metadata is also unsupported. Decide which sections remain supported product behavior, then implement or formally retire them. Existing reconsolidation code is not evidence that all provider-driven graph edits were ported.
- **Shutdown does not drain work.** `daemon/mod.rs:293` detaches connection threads, `admin.rs:270`/`:273` detaches dream workers, and `finish_shutdown` joins only the accept loop. Introduce daemon-owned handles/cancellation and drain work before releasing ownership/removing discovery. This is especially relevant to update/migration coordination.
- **Service/discovery contracts also need alignment with ADR 0009/0012.** The CLI exposes `service start/stop` instead of the specified top-level `start/stop`; `service stop` contacts systemd rather than issuing authenticated shutdown to any discovered foreground instance. The stdio adapter autostarts only with an opt-in flag and only for missing discovery, then spawns a raw daemon (`stdio.rs:60`, `:83`) rather than starting the per-user service. Discovery/doctor checks use TCP reachability, and hooks also use PID existence, without the authenticated process-instance comparison required by ADR 0009. Implement the specified lifecycle or explicitly amend the public contract; comments saying the current behavior is documented do not resolve this discrepancy.
- **Upgrade receipt crash window.** `upgrade.rs` writes the complete journal before `write_receipt` in both fresh and resumed paths. `already_complete_report` accepts a missing receipt. A crash or I/O failure in that interval makes the audit receipt permanently optional on retry, contrary to the helper's claim that it is durable before completion. Make finalization retryable or persist the receipt before terminal completion. This is source inspection, not an injected crash reproduction.
- **Release source scope is narrower than a normal updater.** `hiero update` requires a local release directory. The remote channel/feed resolution story remains separate; document it as such rather than implying release settings drive automatic retrieval. Preserve the existing explicit signature-policy decision.
- **Small documentation fixes:** rustdoc warns about unescaped `argv[0]` in `app.rs:25` and a public link to private `embedded` in `daemon/assets.rs:9`. Root AGENTS.md still describes the Python stack and Python-only verification; align contributor instructions when Rust becomes authoritative.

## Cross-check against Sonnet's independent report

Reviewed `docs/sonnet-report.md` against the same source commit. Sonnet's default-feature 579-test result and this report's all-feature 583-test result are compatible, not competing counts.

| Sonnet item | Adjudication and additional coverage |
| --- | --- |
| 2.1 MCP wiring | Confirmed; already finding 1. Its statement that the console is functional is contradicted by findings 2–4 and the process/HTTP probes. |
| 2.2 missing recall contract | Confirmed; already finding 13. Follow ADR 0011; no additional owner decision is necessary to implement the accepted requirement. |
| 2.3 single-provider limitation | Confirmed; already findings 5–6. Do not amend down to single-provider merely to match current code. Also, the current daemon does not open DreamService at startup: this refusal happens when opening the dream service on a run, not at daemon startup as Sonnet states. |
| 2.4 service command differences | Confirmed; already in lifecycle follow-ups. Sonnet's assumption that systemd SIGTERM invokes the current handler is unsupported: `ctrlc = "3"` has no `termination` feature enabled. Include SIGTERM process testing and handler setup in lifecycle work. |
| 2.5 waived CSRF fixture marker | Useful new documentation action. Annotate the compatibility index with ADR 0012's amendment and explicitly distinguish historical frozen cases from current Rust expectations. Do not alter old fixture bytes just to hide the distinction. |
| 3.1 domain commit before audit/phase completion | **New confirmed durability gap.** `dreaming.rs:1195`, `:1282`, and `:1422` commit before `complete_deterministic_phase`; that helper updates the phase on another connection and then appends audit separately (`:1456`). A crash can leave durable mutations without their durable completion record. Fix by writing the completion audit and phase state through the same transaction as the domain changes. The strongest direct requirement is ADR 0005's immutable dream audit; this is not proof that structured-rule approval itself lacks transactional audit. |
| 3.2 exhausted pair budget consumes skipped work | **New confirmed correctness gap.** `dreaming.rs:1406` applies a pair only while budget remains, but `:1415–1420` stamps every selected activation consumed. Three distinct advisory crystals with three eligible pairs and budget one lose the other two pairs. Merely leaving all activations unconsumed would replay the first pair; use durable per-batch pair progress and transactional consumption. |
| 3.3 no Rust-to-Rust upgrade path | **New required lifecycle work, with scope qualification.** Editing the initial create script before any Rust release is not itself corruption of a released schema. However, `open_migrated` only creates fresh/current databases, and `run_upgrade` treats any complete journal as terminal without comparing its target to a later target. Introduce ordered Rust upgrades before the planned additional durable tables, and test a second upgrade after a completed Python cutover. |
| 3.4 interpolated series predicate | No demonstrated injection: `validate_slug` immediately guards interpolation. Keep that strict boundary and add regression coverage through one validated predicate builder. Do not promise parameter binding without verifying support in the pinned backend. |

These additions are covered by the linked [remediation implementation plans](superpowers/plans/2026-09-05-rust-port-remediation.md). Sonnet's report remains unchanged.

## What to retain

The library/binary boundary is useful, and typed errors are generally appropriate. Deterministic terminology is kept separate from advisory recall. Semantic generation identities include tokenizer identity, which provides the mechanism needed to invalidate byte-fold generations. The durable job and migration state machines already have meaningful failure-path tests. The provider gate correctly refuses silent deterministic substitution. These are foundations to integrate and correct, not reasons to start over.

The applied `good-rust-practices` skill's relevant rules are “Tests must exercise error paths,” “Concurrency — match the primitive to the access pattern,” and using Result for recoverable failures. Here their practical consequences are testing real entry points and rollback outcomes, owning worker lifetimes, and propagating recovery failures. I would not spend this phase changing every clone, introducing more generics, or replacing synchronous code with async for style.

## Recommended implementation sequence

1. **Correct ownership and recovery first:** shared data-root locking/classification, parse-only runtime config loads, worker shutdown, updater failure handling, stable credentials, retryable upgrade finalization.
2. **Complete the agent path:** all frozen MCP dispatch, daemon-proxied CLI mutations, service construction, actual persisted end-to-end workflows and domain errors. Return ADR 0011's deterministic contract independently of ranking.
3. **Complete the console path:** bootstrap commands/exchange, browser-compatible guards, every supported view/action, real embedded-bundle browser tests.
4. **Complete dreaming:** workflow execution plan, enabled checks, per-workflow providers/models, scheduler and successive capped drain cycles, supported output sections, auditable failures.
5. **Complete memory and semantic RAG together:** correct and connect real tokenization, generation invalidation, worker execution/reconciliation, daemon recall integration, and relevance evaluation. Exercise imported RAG documents and learned memory through the shipped application. The owner requires the semantic lane; do not substitute an FTS-only release or resolve the gap by disabling semantic retrieval in documentation.
6. **Run the cutover rehearsal and owner question-list review:** actual release artifact, real browser and MCP client, supported upgrade/recovery scenarios, managed restart/update/rollback, and end-to-end memory and semantic RAG retrieval checks. Both memory and the semantic lane must pass; FTS-only operation is not completion. No extra attestation/gate tooling is needed.

Each slice should have acceptance criteria expressed as user-visible behavior and durable effects. Replace “all advertised schemas load” and “fixture-shaped response returned” as completion gates with “the advertised operation performs the intended work through the shipping binary.”
