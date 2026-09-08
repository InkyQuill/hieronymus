# Authority ingress v1

Ordinary MCP submissions have agent authority, regardless of `actor` labels, quoted
transcripts, or claimed source roles. Unknown draft fields are rejected. Legacy
`hieronymus_termbase_approve` and `hieronymus_rule_crystal_archive` report unverified
origin, including for migration-protected global rules. Normal structural operations
and correlated recall-outcome relevance feedback retain their separate behavior.

The active Rust schemas are in `compatibility/rust/authority-ingress-v1.json` and
`McpRegistry::embedded()`. The historical Python snapshot remains frozen.

## Ordinary MCP operations

- `hieronymus_order_register`: `series_id`, `snapshot`, optional `timeline_id`,
  `expected_revision` (the **timeline** revision). Imports the exact v1 order manifest
  and returns timeline/revision, manifest evidence ID and ordered position IDs.
- `hieronymus_evidence_capture`: `series_id`, `snapshot`, `kind` (`source_passage` or
  `aligned_rendering`), `selection: {start,end,expected_text}`, and the strict
  `EvidenceBindingV1` object. Returns the immutable `reference`, source identity,
  selected text and source paragraph bounds. Offsets index the entire UTF-8 document.
- `hieronymus_decide`: `DecisionDraftV1`, for Activate/Replace/Scope/Archive operations.
- `hieronymus_correct`: the same draft with `operation: {"Correct":{"intent":...}}`.
  A receipt reference cannot turn the ordinary MCP principal into a user principal.

A snapshot is either `{kind:"file",path,expected_hash}` (a **new explicit import** of
all file bytes), or `{kind:"retained",evidence_id,expected_hash}` (immutable stored
content, without reopening the historical path). There is no inline-document form.
`expected_text` asserts the selected bytes; it never supplies source evidence.
Alignments use the target document selection and link the source capture through
`binding.aligned_source_id`; the producer derives source paragraph bounds.

Create a series and concept, import an order manifest, start a session with the
returned timeline/scene, capture source and aligned target anchors, then propose a
candidate using `hieronymus_termbase_propose`'s additive `concept_id`. Read current
`resulting_revision` through a coherent contract/recall operation before deciding.
The decision's `expected_revision` is the **series authority** revision, not the
manifest's timeline revision. No adapter refreshes a stale value.

`DecisionDraftV1` contains `version:1`, UUID `decision_id`, `expected_revision`,
`evidence_refs`, `series_id`, nullable `concept_id`, normalized `source_language`,
nullable `target_language`, `applicability`, `operation`, optional `receipt_ref`, and
optional `session_id`. Operation/intent enums use the existing external JSON tags
(`Activate`, `Correct`, `Rendering`, `Fact`, etc.). IDs must be positive.
`hieronymus_feedback` accepts either `correction_text` or a `decision_draft`, together
with `session_id`. Prose records an unverified tentative signal; a typed draft must
bind that same session and passes through `hieronymus_correct`.

## Separate local user routes

The same-account shell is trusted. The daemon maintains three distinct 0600 local
credentials under its config root. Ordinary MCP uses `daemon.token`.
`console.token` authorizes only the console launch-grant path;
`host-event.token` authorizes the host event route. These credentials are never
returned by MCP/status, placed in generated bundles, or used as receipt IDs.

- `POST /auth/launch-grant`: `Authorization: Bearer <console credential>` and valid
  Host. The CLI already uses `DaemonClient::with_local_credential(Console)`.
- Existing grant exchange, fragment bootstrap, single-use grants, exact mutation
  Origin checks and daemon-lifetime session cookies remain in force.
- `POST /api/authority/correct`: authenticated browser session cookie and exact
  Host/Origin; accepts `UserCorrectionV1` below.
- `POST /authority/host-event`: `Authorization: Bearer <host-event credential>` and
  valid Host; accepts the same DTO, requires an existing matching session and prose
  `text`, and forbids structured fields. An independent host stdin handler must
  obtain the actual event text/session outside model tool arguments. A genuine new
  delivery needs a new event ID, even when its text repeats; a delivery retry keeps
  its original ID and exact immutable context.

`UserCorrectionV1` fields are `version`, UUID `decision_id`, `event_id`,
`expected_revision`, `series_id`, nullable `session_id`, nullable language fields,
nullable `applicability`, `selected_sources: EvidenceRef[]`,
`selected_claims: [{id,revision}]`, nullable `selected_rule: {id,revision}`, and
exactly one of `text` or `structured`. The server reloads selected source bytes,
concept, rule forms and claim identity. It never selects one of several claims or
occurrences implicitly. Missing scope/language/selection and changed source hash
produce distinct tentative details. Actual stale revisions return RevisionConflict.

Structured forms are `{kind:"rendering",canonical,approved_variants?,forbidden_variants?}`,
`{kind:"invalidate"}`, or `{kind:"qualify",qualification}`. Rendering preserves
independently approved/forbidden forms unless explicitly supplied; the old canonical
is not retained just because it occupied an approved-form storage row.

`POST /api/authority/selection` uses the same browser guard. It is a read transaction
accepting `series_id` and optional `target` (typed `ClaimTarget`, e.g.
`{source:"short_term",id:123}`), `source_evidence_id`, and `rule_id`. It returns
`expected_revision`, the selected evidence/rule/claim target languages, **all** bound `claims` with text,
claim IDs/revisions and applicability, optional exact `source`/binding, optional rule
ID/revision/canonical, and `source_inspection:true`. This read mints no authority.
The UI must display multiple claims and require a deliberate selection. For rendering,
select an actual source capture; an opaque generic memory row ID is insufficient.

## Results and stored provenance

Domain results retain `Applied`, `Replayed`, or `Tentative` with the domain receipt,
including resulting revision, affected rule/claim revisions, effective applicability
and exclusions, origin, and consolidation job ID. A replacement receipt may list both
the superseded predecessor and its new active rule; do not reuse the predecessor
blindly. Validation can require the accepted `decision_id`.

An unresolved authentic event has no invented domain operation. Its response is
`{status:"tentative",decision_id,origin_receipt,reasons,detail,authority_changed:false,
resulting_revision,consolidation_job_id}`. In one immediate transaction it stores
an immutable origin, a typed `UnresolvedSignalV1` in the existing v5 decision record,
the exact result/reasons, one pending consolidation job, and an incremented series
revision. No rule or claim changes. Exact input/principal replay returns the stored
result before revision checking and creates no duplicate work. A new delivery must
observe the new revision; changed decision/event identity conflicts rather than
rebinding the signal. Tentative IDs cannot satisfy `required_decision_id`.

The stored canonical variant has `kind:"unresolved_signal"`, `version:1`,
`decision_id`, `origin`, full `text`, full strict submitted `context`, `reasons`, and
`detail`; it contains no invented `operation`. For prose, `context_text_elided:true`
means the duplicate `context.text` is omitted and reconstructs exactly from `text`.
False or absent preserves the context verbatim, including old v1 rows and structured
inputs with null text. Origin storage still retains the full original text/context.
The existing correction worker leases
this job, verifies the origin text/context/hash, and includes the full signal and
reasons in its bounded gathering context. Canonical unresolved input is capped at
448 KiB using a limit shared by ingress and worker, reserving 64 KiB within the
unchanged 512 KiB total budget. The final serialized worker projection is checked
against that total as well. One copy of the full 64 KiB raw text fits even with
worst-case JSON control-character escaping; oversized additional context rejects
atomically, without truncation. Resolved decision requests keep their 16 KiB bound. Worker output retains ordinary Dream/learned policy; it cannot
turn unresolved intent into explicit user authority. Empty completion finishes that
gathering attempt while leaving the original tentative signal/reasons available. Domain
errors use HTTP 409 with a typed `error` (including `RevisionConflict.current_revision`);
malformed JSON/unknown structured fields use HTTP 400. Authentication failures are 401
and browser Origin failures 403. MCP domain errors preserve `isError:true` and the
complete result envelope with a typed `structuredContent.error`.

Resolved origins retain the full original event text, hash, principal, session and
operation/evidence/revision context. Immutable `user_event` evidence retains the full
draft plus exact parsed UTF-8 token spans and decoded values; receipt binding reparses
and compares these against the original text. Agent requests cannot mint either user
origin kind. No provider extraction or implicit revision rebase participates.

The HTTP/stdio/console bridge tests establish server enforcement only.
Task6c adds actual UserPromptSubmit stdin delivery and the dedicated console form; installed
Claude/Codex/zCode acceptance remains a separate qualification gate.

## Installed prompt delivery and console workflow

A clear host correction is applied by the trusted endpoint immediately, without
waiting for a model or provider. Its hook output supplies `required_decision_id`
for subsequent model reads/validation. Ordinary MCP `hieronymus_correct` remains
an Agent operation and cannot redeem a host or console origin receipt. The phrase
“bind an already minted receipt” describes the trusted endpoint's internal work.

`hiero agent-hook bind-context --data-root <root>` reads one versioned JSON object
from stdin. The object contains `version:1`, `host` (`claude`, `codex`, or `zcode`),
`host_session_id`, existing numeric Hieronymus `session_id`, `series_id`, observed
`expected_revision`, nullable language/applicability fields, `selected_sources`,
`selected_claims`, and nullable `selected_rule`. Selection shapes match
`UserCorrectionV1`; event IDs, prompt text, structured correction, and actor fields
are forbidden. The installed workflow supplies these values from actual session
and selection results. Binding validates the existing session/series/languages,
revision, immutable source references and selected claim/rule revisions; it never
refreshes a stale revision, invents an identity, or grants user authority.

`hiero agent-hook user-prompt-submit --host claude --data-root <root>` (substitute
`codex` or `zcode`) reads the independently supplied host stdin envelope. It requires
`hook_event_name:"UserPromptSubmit"`, nonempty `session_id` and `prompt`, and matches
that host/session to the saved binding. Other host metadata is not treated as
selection or authority. Contexts and deliveries are atomically stored as 0600 files
under `host-contexts/` and `host-deliveries/` in the application root. Credentials
remain in the existing separate local credential files, never in these commands,
plugin JSON or output.

Each invocation creates a new UUID and saves the entire selected request before
HTTP. Repeated identical prompts and reused Codex `turn_id` values are distinct
invocations. `hiero agent-hook retry-delivery --delivery-id <uuid> --data-root <root>`
resends only the saved request after an uncertain failure; an acknowledged delivery
returns its saved result. Retry must not be implemented by invoking
`user-prompt-submit` again. Hosts without stable per-delivery identifiers cannot
prove whether an automatic re-invocation is a retry or a genuinely repeated event;
this implementation makes that limit explicit instead of deduplicating by text.
Failures retain the delivery ID for exact retry and exit nonzero. No daemon starts
implicitly. Rebinding a session applies only to future deliveries.

The common input fields follow the [Claude hook reference](https://code.claude.com/docs/en/hooks)
and [Codex UserPromptSubmit input schema](https://github.com/openai/codex/blob/main/codex-rs/hooks/schema/generated/user-prompt-submit.command.input.schema.json).
Output uses `hookSpecificOutput.hookEventName:"UserPromptSubmit"` and
`additionalContext`, containing the actual result and accepted dependency ID when
applied. Recognizing an envelope is not native-host qualification; Task7 owns
supported installation, independently observed host delivery and S1–S7 transcripts.
zCode's claimed Claude-format compatibility still requires that actual acceptance.

The console now has a dedicated “Correct a rendering” entry and “Correct this
memory” on supported bound memory records. `POST /api/authority/options` lists
actual books and immutable source occurrences, optionally restricted by a typed
memory target or actual rule ID. It is guarded like selection and mints nothing.
Current-rule choices evaluate the selected immutable occurrence's concrete position,
scope and viewpoint with shared applicability and effective exclusions. A retained
A outside a chapter override is not offered inside current B's chapter. Unknown or
multiple viewpoint context is explicitly `context_unresolved:true`; the form withholds
current-rule selection and submission until a resolved occurrence is chosen. Direct
selection by an excluded rule ID is rejected as well.

Selection merges bound normalized language identifiers from source evidence, rules,
and claim targets (short-term session, crystal pair, or facet language), rejecting
conflicts or explicitly empty values. It uses series defaults only when no selected
identity stores language context. A facet-only target has no target language pair and
returns null rather than inventing one. Registered nondefault pairs survive options,
selection and correction unchanged.
The user chooses the occurrence/current rule or one exact claim, then the existing
selection route freezes the displayed revision/context. Submission reports applied,
tentative or conflict; retries retain the original request, while an explicit refresh
loads a new selection. The applied rendering is displayed immediately. The old
Renderings view is labeled historical source inspection because its `strict_terms`
IDs and rows are not the current `term_rules` authority; these IDs are never aliased.

Memory views retain source records and explicitly label their status as record lifecycle,
not claim validity. After applying a rendering, the frozen old choice is labeled
previous rendering and the result states the current rendering.

Task7 first-session bootstrap handles a missing binding read-only: actual host/session
identity and an actionable bind-context contract appear in hook additionalContext.
The response says binding_required and explicitly says the current prompt was not
retained/applied. No domain session, origin, decision or delivery is invented. A
subsequent genuine prompt can apply after explicit binding from actual MCP outputs.
Corrupt or inaccessible existing bindings remain errors, not bootstrap success.
