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

Capture relevance is determined before saving text or sending a correction request. In **Settings → Ingest → Prompt relevance**, a saved TypeSafe API key makes Jev the primary classifier. One request asks two independent Noul questions: whether the message carries authorial/literary content, and whether it requests software/infrastructure work. The message is eligible only when literary probability meets the minimum and technical probability is at or below the maximum. Successful negative or uncertain answers are skipped; they never fall back to the word filter.

Without a key, the existing conservative Russian/English word filter is used. Exact supported correction grammar takes precedence within that local filter, so quoted renderings and qualifications may contain technical words. On a transport/HTTP failure, malformed response or unusable configuration, the local filter is the fallback. Skipped results include a text-free relevance diagnostic with the method and, when applicable, a generic fallback reason. The local heuristic can miss relevant messages or accept unrelated ones; Jev also remains probabilistic. A relevance result confers no authority: the server still validates selection, scope, identity and revision. Ordinary autonomous agent observation capture remains available through scoped MCP operations.

Jev receives only the current user message, not the conversation history, database, selected evidence, session identifiers or project paths. An unbound hook checks the CWS project before classification; a paused hook skips it. Classification runs outside the conversation lock, so an invocation that already passed its pause check may send/finish its Jev request after unbind. Pause is rechecked under the lock before any durable retention. Requests have a 5-second default timeout, a 64 KiB response limit, and no inline retries; 401, 429, 529 and other non-200 responses use the local fallback.

The optional private `relevance.conf` under the configured data root contains:

```toml
api_key = "YOUR_TYPESAFE_API_KEY"
model = "jev-1.13.0"
minimum_relevance = 0.85
maximum_technical = 0.15
timeout_seconds = 5
```

Use the settings form to create it with private permissions, or protect a manually created file with owner-only permissions (`chmod 600` on Unix). Blank key submissions preserve the saved key; “Remove saved Jev key” explicitly restores local classification. Saved keys never appear in settings responses. The endpoint is fixed to `https://api.typesafe.ai/v1/systemone`; redirects are not followed. The pinned model and initial thresholds are configurable, but these thresholds have not been calibrated on a representative project corpus. TypeSafe documents stronger English than non-English accuracy; qualify Russian content before depending on it.

Protocol sources: [TypeSafe API](https://docs.typesafe.ai/api), [Noul](https://docs.typesafe.ai/primitives/noul), [model/language support](https://docs.typesafe.ai/models), and [known limitations](https://docs.typesafe.ai/model-jaggedness/jev-1.13). For an explicit synthetic live check (sends four RU/EN sample messages, requires a key; never uses real manuscript data), run `CARGO_BUILD_JOBS=2 cargo test --all-features --locked live_jev_synthetic_relevance -- --ignored`. Supply `TYPESAFE_API_KEY` securely in the process environment; missing credentials fail. Real Jev and Ollama service probes are local-only: CI explicitly skips them, and they reject execution when CI or GITHUB_ACTIONS is set before reading keys or contacting the service. CI provider coverage uses mocks and disposable loopback servers. Mock/transport checks do not establish real model accuracy.

A skipped result has status:"skipped", retained:false and authority_changed:false. No full-text delivery is created and no binding_required instruction is injected for that message. Existing bindings do not make technical messages relevant.

To stop capture for one conversation, run `hiero agent-hook unbind-context` with stdin `{"host":"codex","host_session_id":"actual-host-session-id"}`. Repeating it succeeds. A private, text-free pause marker suppresses subsequent capture and bootstrap until explicit bind-context. Bind, unbind and new capture share one persistent per-conversation OS lock under host-locks; lock files are never unlinked. Pause waits for an already saving capture to finish. Transport of an already saved delivery and explicit retries remain independent of pause. It leaves other conversations and immutable pending deliveries unchanged.

An eligible delivery is saved privately before transport so a lost acknowledgement can be recovered by `retry-delivery --delivery-id UUID`. Definitively rejected deliveries retain the original text/context for diagnosis and immutable manual retry; they are not automatically purged or replayed. They have no authority effect. Delete their exact private JSON file under the configured host-deliveries directory if retention is unwanted; this removes local recovery only, not a server-side receipt. Acknowledged deliveries also retain their receipt. No bulk purge occurs during unbind or upgrades.

## Session termination and recovery

Generated Codex and Claude-compatible bundles register `SessionEnd` and
`SessionStart` alongside prompt capture. These are lifecycle events, not the
per-turn `Stop` event: see the [Codex hooks reference](https://developers.openai.com/codex/hooks)
and [Claude hooks reference](https://code.claude.com/docs/en/hooks#sessionend).
The installed handler receives the host's JSON envelope on stdin:

```text
hieronymus-agent-hook session-end --host codex
hieronymus-agent-hook session-start --host codex
```

Only `hook_event_name: "SessionEnd"` with its actual nonempty `session_id` can
terminate the conversation's saved binding. Missing bindings skip; foreign host
identity or bindings with mismatched domain ownership cannot complete another
session. Native event commands must be invoked by the host, not fabricated by a
model. Completion goes through authenticated daemon discovery and the existing
session-complete tool; hooks never start a daemon or write its database directly.
The tool's optional `expected_series_id` and `expected_last_activity_at` must
both be supplied for conditional completion. The SQLite transaction orders
completion against memory capture: activity changing after observation rejects
completion, and capture after completion is rejected.

A private `host-endings` journal records the bound snapshot and acknowledgement.
Repeated events are idempotent. Transport failure reports `pending` and preserves
the original intent rather than claiming completion. Resume cancels a still-active
session's pending termination under its conversation lock. Completed/dreamed
sessions, including a lost completion acknowledgement, remain paused and require
a fresh active session/binding; bind-context rejects inactive sessions. Explicit
unbind pauses remain effective. Bind/unbind invalidates obsolete termination
snapshots, while previously saved prompt deliveries keep their immutable content.

After a crash without a termination event, or on a host/version lacking these
hooks, the operator must stop the host first and explicitly recover one known
binding. Supply its actual host and memory-session IDs, not guessed IDs:

```text
hiero agent-hook recover-session --host codex --json
```

Its stdin object is `{"host_session_id":"actual-host-id","session_id":123,
"confirm_abandoned":true}`. Recovery uses only that current binding, the observed
activity guard and the running daemon. There is no idle timeout, bulk recovery,
or implicit closure of live sessions. A changed binding/session must be inspected
and explicitly rebound before a new recovery attempt. Versions without native
hooks can also explicitly call `hieronymus_session_complete` through MCP.

The lifecycle regressions exercise actual CLI stdin and authenticated local daemon
transport with disposable fixtures. They do not establish native acceptance on
every installed host/version; previously recorded prompt-hook qualification does
not qualify these newly added lifecycle hooks. Changed generated hook commands
require the host's normal review/trust process; no blanket trust is granted.
