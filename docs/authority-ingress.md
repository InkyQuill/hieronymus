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
`expected_revision`, registered default languages, **all** bound `claims` with text,
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
`{status:"tentative",origin_receipt,reasons,detail,authority_changed:false}`. It is
stored immutably but cannot subsequently be rebound to a different context. Domain
errors use HTTP 409 with a typed `error` (including `RevisionConflict.current_revision`);
malformed JSON/unknown structured fields use HTTP 400. Authentication failures are 401
and browser Origin failures 403. MCP domain errors preserve `isError:true` and the
complete result envelope with a typed `structuredContent.error`.

Resolved origins retain the full original event text, hash, principal, session and
operation/evidence/revision context. Immutable `user_event` evidence retains the full
draft plus exact parsed UTF-8 token spans and decoded values; receipt binding reparses
and compares these against the original text. Agent requests cannot mint either user
origin kind. No provider extraction or implicit revision rebase participates.

The HTTP/stdio/console bridge tests establish server enforcement only. Actual
UserPromptSubmit stdin delivery and the dedicated console form are Task6c; installed
Claude/Codex/zCode acceptance remains a separate qualification gate.
