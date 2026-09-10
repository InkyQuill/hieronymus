# Compatibility freeze

Historical Python application and later Rust-port tooling have separate
snapshots. See [the archive policy](../docs/archive/python-v0.7.0.md).

The checked-in manifest, snapshots, and fixtures are the reviewed Python
reference boundary for the Rust migration. Current Rust plans and accepted
ADR amendments govern behavior; these snapshots are historical inputs, not a
Python parity release gate. When intentionally maintaining the retained Python
reference, its read-only aggregate check is:

```bash
uv run --no-cache --no-sync python -B -m tools.compatibility.check
```

When a reviewed Python contract intentionally changes, regenerate only the
affected inventory with:

```bash
uv run python -m tools.compatibility.<inventory_module> --write
```

The write form requires reviewed manifest and fixture changes in the same
change. A changed or removed contract also requires an ADR; generated output
must not be accepted merely to make the gate pass.

Every contract records three independent release-state fields:

- `last_python_release` is the nonblank frozen Python reference in which the
  contract exists;
- `first_rust_release` stays `null` until a real Rust implementation ships;
- `implementation_status` is `outstanding` or `implemented` and must agree
  with `first_rust_release`.

Implementation state does not replace disposition. `preserve`,
`intentionally-change`, and `remove` describe the approved migration outcome;
`outstanding` and `implemented` describe whether Rust has delivered it. The
current freeze therefore reports every contract as outstanding and does not
claim that a Rust implementation exists. The aggregate report derives and
lists implemented, changed, removed, and outstanding contract IDs and counts
from the manifest rather than using frozen totals.

Python pytest ownership and frontend Vitest ownership are separate typed
collections. Each public case names specific contract IDs; implementation-only
cases require a concrete reason. Frontend collection requires Bun and fails
explicitly when it is unavailable, so the canonical gate never silently omits
frontend cases.

This freeze is the finite reference input for the future Rust differential
harness. That harness will replay the same fixtures independently against the
Rust candidate and compare only contract-approved normalization boundaries.
The Python reference records migration parity; it is neither a rollback path
nor a commitment to support Python in production after cutover.

## Versioned Rust expectations

`compatibility/rust/` holds separate, versioned expectation files for
ADR-backed Rust response deltas. They never replace or regenerate the frozen
Python snapshots and fixtures: the Python boundary stays exactly as reviewed,
and a Rust delta is a new file that names its ADR, states the delta, and pins
the changed response shape with explicit expected subsets.

Precedence follows each file's ADR. For `rust/recall-v2.json` (ADR 0011), the
`hieronymus_recall` transport DTO is
`{recall_id, deterministic_contract, results, warnings}`: the deterministic
contract is computed from the query/source context before any lane fusion,
returned separately from the ranked `results`, and serialized whole even when
`limit` removed every advisory hit. The Python fixture's bare ranked list
remains the frozen reference for the Python tool; Rust consumers read the
versioned delta. Since task C5 the same file also pins the `warnings` rule:
`semantic_lane_unavailable` is emitted whenever required semantics did not run
over the current corpus — an unarmed query lane included, which the pre-C5
expectation recorded as a silent supported mode. Warnings are pinned by kind
(`warning_kinds`); the reason text belongs to whichever semantic service is
attached and is deliberately not frozen.

### Semantic RAG search (`rust/rag-search-v2.json`, ADR 0013 / review finding A5)

`hieronymus_rag_search` keeps the frozen envelope — the bare row array the
frozen `hieronymus_rag_searchOutput` schema describes, with the same row keys
— and changes two things. It now serves the same armed semantic lane plus
reciprocal-rank fusion `hieronymus_recall` runs (a session-less series+query
search, no synthetic task session), so `score` is the RRF score over both
lanes rather than the raw FTS score and `rank_reason` distinguishes them
(`rag semantic match` for a semantic-only row). And it requires the semantic
service: absent, acquiring, rebuilding, or failed semantics is a tool error
carrying the service's own actionable reason, where the pre-C5 tool answered
with lexical FTS rows that no caller could tell apart from a complete hybrid
answer. A ready service over a series with no indexed chunks stays an empty
success. The mandatory-semantics half of that rule is the owner's M2/S2
completion contract, not a reinterpretation of ADR 0013's FTS5-only release
baseline: that baseline governs what a release may ship, not what a tool
advertised as semantic RAG search may silently answer.

`hieronymus_concept_proposals_list` has a documented scope delta of the same
kind: the Rust tool returns only the strict half of the Python response — the
safe DTO projection of the pending `strict_concept_proposals` rows. The
Python tool also merges recent dream-audit concept-suggestion payloads into
the same list; that dream-audit merge lands with the dreaming plan, so until
then Rust consumers see only the strict proposals.

`hieronymus_rag_import` carries additive Rust-only keys with no versioned
expectation file, because the frozen Python boundary pins no response shape
for them: `semantic_rebuild_job` (task S2) and, alongside it,
`semantic_indexing` plus the conditional `semantic_indexing_error` (task C4).
`semantic_indexing` is always present and is one of `queued`, `owed`, or
`not-required`; `semantic_rebuild_job` is a durable job id exactly when the
value is `queued` and `null` otherwise — never a structured error object, and
never the controller's internal `rebuild:empty-corpus` marker. Python has no
semantic indexing lane at all, so these keys add to the response and change
nothing a Python consumer reads.

### Web console authentication (`rust/console-auth.json`, ADR 0012)

The frozen HTTP route-cases have no way to launch an authenticated console:
nothing mints or opens a launch grant, and the browser `GET /api/*` routes
reject a request that carries no `Origin`. `rust/console-auth.json` records
the two ADR 0012 (2026-09-03 amendment) deltas that close that gap:

- **Launch-grant fragment transport.** `hiero admin` / `hiero config` mint the
  existing 60-second single-use grant over the bearer-authenticated
  `POST /auth/launch-grant` and open
  `http://<addr>/<page>#launch_grant=<grant>` with `xdg-open`. The grant rides
  only in the URL *fragment* — never a query string, never a log line — and
  `frontend/src/web/lib/bootstrap.ts` scrubs it (synchronous
  `history.replaceState`) before the one-time `POST
  /auth/launch-grant/exchange`. The amendment prohibits query-string secrets
  and waives CSRF tokens; the fragment path is the reviewed transport and no
  CSRF token is restored.
- **Safe-read Origin rule.** An authenticated browser `GET`/`HEAD` with **no**
  `Origin` is served (a top-level navigation into the console), but an
  explicit foreign `Origin` on a read is still `403`, and every mutating
  method, the grant exchange, and the `GET /ws/admin` upgrade still require
  the exact `Origin` `http://<addr>`. Host validation, grant replay/expiry
  checks, `HttpOnly; SameSite=Strict` cookies, and daemon-lifetime sessions
  are unchanged.

The frozen `compatibility/snapshots/` and `compatibility/fixtures/` bytes are
untouched; the new Rust integration coverage lives in
`crates/hiero/tests/console_auth.rs` and `frontend/src/web/lib/bootstrap.test.ts`.

### Current lifecycle and installed evidence

Recall's versioned Rust DTO/semantic-readiness deltas and the console Origin
rules above remain intentional. [ADR 0015](../docs/adr/0015-mcp-protocol-and-transport.md)
requires MCP 2026-07-28 without a silent legacy initialize adapter.
[ADR 0016](../docs/adr/0016-autonomous-story-memory-product-vision.md)
supersedes human-only rule approval as product direction while retaining
explicit-user priority, deterministic authority, evidence and audit. Its accepted
authority design has not implemented that runtime; historical registry lifecycle
cases passing does not establish autonomous correction/viewpoint acceptance.
The [installed rehearsal](../docs/rust-cutover-rehearsal.md) separates package,
real model, browser and native-host outcomes. Frozen inputs remain untouched.
