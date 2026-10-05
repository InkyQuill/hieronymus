# Qualification assets and historical checks

Most tracked contents are frozen records, schemas, fixtures and isolated harnesses
from the Rust migration. Those qualify their exact recorded inputs, not the current
SQLite-vector backend, desktop packages or installed writing-agent workflows.
The directory is excluded from the production Cargo workspace.

## Current dependencies

- Release asset acquisition stores verified model/runtime cache inputs under
  `.artifacts/models/`; an ignored live-model test also uses that location.
- The separate desktop-evidence workflow reads `qualification/desktop-evidence`
  from an explicitly supplied evidence commit. It is current optional evidence,
  not the historical Rust-port records.
- `desktop-environment.sh` is a local resource-limit helper.

The historical harnesses and receipts are archive candidates, not required runtime
inputs or a standing release gate. In particular, the old semantic harness still
uses LanceDB rather than the current SQLite-vector backend. Do not delete the
whole directory while cache and desktop-evidence consumers still reference it.

Keep existing machine-readable receipts and fixtures unchanged. Generated Markdown
projections and old replay instructions are available in local documentation history
and Git; their `tools.qualification` Python orchestration is not in the active source
tree. [Archive policy](../docs/archive/python-v0.7.0.md) identifies the source snapshots.
Do not infer unfinished Rust implementation from a historical manifest field or
require another Python-vs-Rust certification layer.

Ignored `.artifacts/` holds local acquired assets, disposable roots and retained
logs. Do not delete it during a prose cleanup or treat its presence as a passing
check. Never point a harness at a user's real data or service registration.

Current procedures are in [runtime checks](../docs/rust-cutover-rehearsal.md),
[semantic validation](../docs/semantic-validation.md),
[desktop checks](../docs/desktop-qualification.md) and
[host checks](../docs/agent-host-acceptance.md). Contributor/release policy lives in
[AGENTS.md](../AGENTS.md). Report old and current evidence separately.
