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

This freeze is the finite reference input for the future Rust differential
harness. That harness will replay the same fixtures independently against the
Rust candidate and compare only contract-approved normalization boundaries.
The Python reference records migration parity; it is neither a rollback path
nor a commitment to support Python in production after cutover.
