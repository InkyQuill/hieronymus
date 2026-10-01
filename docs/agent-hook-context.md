# Trusted prompt binding (version 1)

`hiero agent-hook --help` and `hiero agent-hook bind-context --help` print this contract without opening a database or reading stdin.

`bind-context [--host claude|codex|zcode] [--data-root PATH]` reads one JSON object from stdin, maximum 1 MiB. An optional --host must equal the stdin host. No cwd, prompt, actor or event field is accepted. Unknown fields are rejected. The binding confers no authority and does not replay the current prompt.

| Field | Type | Required / meaning |
| --- | --- | --- |
| version | integer | Required, exactly 1 |
| host | string | Required: claude, codex or zcode |
| host_session_id | string | Required, 1–512 bytes; actual host hook session_id |
| series_id | integer | Required, positive; actual series ID |
| session_id | integer | Required, positive; actual Hieronymus task session ID, not host session_id |
| expected_revision | integer | Required, nonnegative and below i64::MAX; coherent observed series authority revision |
| source_language | string or null | Optional; use the bound session/evidence language |
| target_language | string or null | Optional; null for a monolingual authoring session |
| applicability | object or null | Optional; copy selected evidence/claim applicability exactly |
| selected_sources | array | Optional, defaults to []; only immutable source_passage references |
| selected_claims | array | Optional, defaults to []; objects with integer id and integer revision |
| selected_rule | object or null | Optional; integer id and integer revision of the observed rule |

A source reference contains `kind:"source_passage"`, integer `id`, string `content_hash` (whole captured UTF-8 content SHA-256), integer `span_start`, integer `span_end` (UTF-8 byte offsets). Obtain it from `hieronymus_evidence_capture`; do not construct it from guessed IDs or current mutable file offsets.

A non-null applicability object has `series_id`, `timeline_id` (integer or null), `volume_key`, `chapter_key` (string or null), `scope_predicates` (string array), `valid_from`, `valid_until` (position IDs or null), `metadata_state` ("Resolved" or "Unspecified"), and `knowledge_gates` (array of objects with viewpoint, known_from, known_until). Viewpoint is "Narrator", "All", or {"Character": observed_id}; endpoints are observed position IDs or null. Copy this object from immutable selected evidence/claims. Missing chronology is not permission to invent a timeline or declare timeless truth.

## Obtain context

1. Use the actual host's UserPromptSubmit `session_id` as host_session_id. For a recognized CWS literary project, a relevant unbound prompt reports it in binding_required. Other projects and unrelated/uncertain prompts skip capture.
2. Use `hieronymus_series_list` and `hieronymus_session_start` for the actual series and task session/languages. `hiero project-context` describes the project binding but does not supply authority revision.
3. Read `hieronymus_recall` with that series_slug and session_id. Its top-level `resulting_revision` is the observed series authority revision for expected_revision. Selected source/claim/rule revisions must agree with the same context; a concurrent change causes binding rejection. Read again for a genuinely future event, never rebase or replay an existing delivery.
4. Pass returned values to bind-context on stdin. For monolingual research with no selected source/claim/rule, applicability may be null and all selections empty. A future accepted message remains tentative when scope is unresolved; it is not an active rule.

The following is a syntax example for a monolingual session. IDs and revision illustrate types only: replace 12, 34 and 5 with actual outputs, and the host ID with the genuine host identity. It is not a valid live binding by itself.

```json
{
  "version": 1,
  "host": "codex",
  "host_session_id": "actual-host-session-id",
  "series_id": 12,
  "session_id": 34,
  "expected_revision": 5,
  "source_language": "ru",
  "target_language": null,
  "applicability": null,
  "selected_sources": [],
  "selected_claims": [],
  "selected_rule": null
}
```

## Capture and stop capture

Capture relevance is determined automatically, locally and conservatively before saving text or sending a correction request. Exact supported correction grammar and Russian/English literary task vocabulary are eligible; technical vocabulary vetoes mixed free-text requests. Exact correction grammar takes precedence so quoted renderings and qualifications can contain technical words. Short status replies and uncertain wording are skipped. This is a deterministic heuristic, not semantic model classification: it can miss relevant messages. A relevance result does not confer authority; the existing server decision protocol still validates selection, scope, identity and revision. Ordinary autonomous agent observation capture remains available through scoped MCP operations.

A skipped result has status:"skipped", retained:false and authority_changed:false. No full-text delivery is created and no binding_required instruction is injected for that message. Existing bindings do not make technical messages relevant.

To stop capture for one conversation, run `hiero agent-hook unbind-context` with stdin `{"host":"codex","host_session_id":"actual-host-session-id"}`. Repeating it succeeds. A private, text-free pause marker suppresses subsequent capture and bootstrap until explicit bind-context. Bind, unbind and new capture share one persistent per-conversation OS lock under host-locks; lock files are never unlinked. Pause waits for an already saving capture to finish. Transport of an already saved delivery and explicit retries remain independent of pause. It leaves other conversations and immutable pending deliveries unchanged.

An eligible delivery is saved privately before transport so a lost acknowledgement can be recovered by `retry-delivery --delivery-id UUID`. Definitively rejected deliveries retain the original text/context for diagnosis and immutable manual retry; they are not automatically purged or replayed. They have no authority effect. Delete their exact private JSON file under the configured host-deliveries directory if retention is unwanted; this removes local recovery only, not a server-side receipt. Acknowledged deliveries also retain their receipt. No bulk purge occurs during unbind or upgrades.
