# Installer and autonomy follow-ups (#36, #37, #47)

Successful shell/PowerShell setup now exposes bootstrap diagnostics, including migration-pending warnings with --no-open. Existing data is not implicitly migrated. The focused orchestration fixture verifies successful warning delivery.

Native path/default qualification passed using published v0.9.3 binaries in [run 36884346338](https://github.com/InkyQuill/hieronymus/actions/runs/36884346338): Linux XDG, macOS Application Support, Windows APPDATA, environment overrides and CLI precedence. NSIS/pkg smoke also passed. No Rust native matrix was rebuilt. Additional legacy-database native fixtures are advisory; their actual results belong in #36/#37.

The proposal queue, admin view/actions, MCP compat reader, provider schema and producer are removed. Historical pending rows become archived when a current-schema database opens. The table, payloads and approved/rejected history remain. No schema shape or active authority is changed; archival only takes a writer lock if pending rows exist. New provider output using the retired key is diagnosed in the audit instead of creating a queue. Direct concept/facet/claim operations and corrections remain available. Frozen Python fixtures stay untouched; the accepted Rust delta is recorded in compatibility/rust/proposals-retired.json.

Regressions cover payload/history/authority preservation, repeated opening, concurrent WAL reads, unavailable retired operations and absence of newly produced proposals. No installed runtime or user database was modified.
