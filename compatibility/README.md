# Compatibility freeze

The checked-in manifest, snapshots, and fixtures are the reviewed Python
reference boundary for the Rust migration. Run the read-only aggregate gate
before submitting a change:

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
versioned delta.

`hieronymus_concept_proposals_list` has a documented scope delta of the same
kind: the Rust tool returns only the strict half of the Python response — the
safe DTO projection of the pending `strict_concept_proposals` rows. The
Python tool also merges recent dream-audit concept-suggestion payloads into
the same list; that dream-audit merge lands with the dreaming plan, so until
then Rust consumers see only the strict proposals.

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
