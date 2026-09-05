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
