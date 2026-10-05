# AGENTS.md

## Project

Hieronymus is a local desktop server providing several types of agent memory for writing and literary translation projects. Source code lives here, while book projects and runtime databases live elsewhere.

The server serves the main web interface. Authors use it to inspect what their agent remembers, add pointers or context, and flag wrong or stale memories; it is not a comprehensive human-maintained project knowledge base. The main flow is to start the server, connect an agent by adding both its MCP connection and Hieronymus skills, verify both, and continue writing in Codex, Cowork or pi. Keep this flow approachable for nontechnical writers.

`hiero` is a convenience CLI for quick tasks. The running server owns its tray presence, directly or through a supervised sidecar, whenever the desktop supports it. Browser authentication is off by default and optional in configuration; this does not disable authenticated MCP access.

## Frontend design

Read [PRODUCT.md](PRODUCT.md) and [DESIGN.md](DESIGN.md) before changing the console. Use the vendored Thoth palette through Hieronymus semantic tokens; preserve both themes and author-facing progressive disclosure.

## Development Defaults

- Use `Pavel Obruchnikov <me@inkyquill.net>` for formal author metadata unless local git config overrides it.
- Prefer small, testable Rust modules with explicit domain boundaries.
- During v0, preserve process logs and stdout/stderr in readable diagnostics by default; do not silently discard background errors. Log files must retain private-file ownership checks and redact credentials and browser grants. Ollama needs bounded context; cloud models should use their advertised capacities rather than arbitrary local caps, with thinking disabled for structured extraction when supported.
- Keep strict terminology logic deterministic. Fuzzy memory and semantic recall must never silently override approved termbase entries.
- Do not write tool source code into `/home/inky/Yandex.Disk/Translation`.

## Current Stack

- Rust 1.98 workspace: `crates/hieronymus` owns SQLite/domain operations; `crates/hiero` owns CLI, daemon, MCP and distribution.
- SQLite with FTS5; exact SQLite vector search, ONNX and the pinned multilingual tokenizer provide mandatory semantic retrieval.
- Svelte 5 console in `frontend`; Bun 1.4.0 builds and tests it. The release binary embeds its production assets.
- Authenticated local MCP HTTP (revision 2026-07-28) and a stdio adapter.
- Bun TypeScript in `scripts` owns release acquisition and metadata validation. No Python tooling is required. The previous Python implementation is archived on `stale/python-v0.7.0`.

## Verification

Run relevant focused tests while editing, then these before claiming Rust implementation work is complete:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked
bun test scripts/*.test.ts
cd frontend
bun run typecheck
bun run test
bun run build
```

Real model and installed-artifact tests are explicitly ignored by default. Supply their documented disposable fixture inputs and run them explicitly when qualifying those paths; missing inputs must fail. See `docs/rust-cutover-rehearsal.md`. Passing synthetic provider or transport tests does not establish native agent-host acceptance.

Current product contracts and accepted decisions govern product behavior; start at [docs/README.md](docs/README.md). Preserve frozen compatibility fixtures as historical evidence; do not introduce a Python parity release gate. An accepted ADR describes intended behavior, not proof of runtime or native-host acceptance.

## Release checks and blockers

- Only P0 issues block a release. P1, P2, P3, missing native-host evidence, smoke-test failures, lint/documentation findings, and incomplete platform qualification are warnings; record actionable failures in GitHub issues with reproduction details and the relevant run/log link. Reuse an existing issue for the same problem. Never silently label a failed or unrun check as passed.
- Run focused, inexpensive checks first. Keep the verification commands above as the verification checklist, but report non-P0 failures as warnings rather than starting an indefinite fix/rebuild cycle or withholding the release.
- Use `CARGO_BUILD_JOBS=2` for local Rust checks: many concurrent debug linkers for the retrieval stack can exhaust memory and spend minutes swapping. An interrupted or unrun full check must be reported honestly as a verification warning with an issue, not restarted indefinitely.
- Build native binaries once and retain the artifacts. Installer, packaging, documentation, and CI-only fixes must reuse those binaries when their build inputs are unchanged. Rerun the affected check or packaging job, not the entire platform matrix. Rebuild only when binary inputs change or the required artifact is missing.
- Native installation checks and the separate evidence workflow are advisory. A missing optional installer or evidence record must not prevent publication of the available working artifacts; disclose the limitation in release notes and an issue.
- Keep release automation small. Do not introduce extra qualification layers, duplicate gates, or new mandatory evidence scaffolding without an explicit product need. These rules supersede stricter release-blocker language in older plans and ADRs.
- Artifact integrity checks and safeguards against corrupting user data remain runtime requirements. Do not make a checksum mismatch executable or bypass data ownership merely to turn a release check green.

## Default paths

- Use the operating system's standard configuration directory by default: `$XDG_CONFIG_HOME/hieronymus` (fallback `~/.config/hieronymus`) on Linux, `~/Library/Application Support/Hieronymus` on macOS, and `%APPDATA%/Hieronymus` on Windows.
- `--data-root` overrides `HIERONYMUS_DATA_ROOT`, which overrides the platform default. Keep the CLI, installers, desktop launchers, and service registrations consistent. Never persist development or temporary fixture paths into a user's real service registration.
- Tests that install services or desktop registrations must use disposable configuration, data, and registration directories and must not contact the real user service manager.

## Working with Docs

- Write for a person with a concrete task. Lead with what they can do or need to understand; define domain terms before using them. Keep author instructions separate from protocol, build and release details. Use the repository's English documentation language consistently.
- [README.md](README.md) owns the introduction and installation entry point; [docs/README.md](docs/README.md) is the single navigation index. [PRODUCT.md](PRODUCT.md) owns purpose and product principles; [DESIGN.md](DESIGN.md) owns interface conventions; [docs/business-logic.md](docs/business-logic.md) explains memory, authority and failure behavior. Usage and technical guides own their procedures. AGENTS.md owns contributor instructions, not a second product specification.
- Give each rule one maintained home. Link to it instead of copying settings, commands, schemas, version pins, test totals or release requirements. Add a document only when an existing guide cannot serve its distinct reader/task; list it in the documentation index.
- Record consequential business-logic decisions in the existing relevant ADR: context, decision, reason/tradeoff, consequences and implementation/evidence limits. Update its main text to the current decision; remove superseded requirements from the active reading path. Git preserves the previous wording. Create another ADR only for a distinct decision that cannot reasonably fit an existing one; do not renumber old ADRs.
- Distinguish accepted policy, implemented behavior, measured results and unresolved work. Verify implementation claims against current code/tests; never infer native-host acceptance, provider accuracy or release readiness from a plan, generated assets, mocks or old receipts. Put actionable unresolved work in an issue or the concise roadmap.
- Before finishing a behavior change, sweep all affected product guides, ADRs, agent resources and links for contradictions. For a documentation consolidation, inventory every project-owned Markdown file, including hidden/local files; classify runtime resources, provenance and frozen evidence separately from reader documentation. Exclude dependencies, build outputs and other worktrees.
- Keep only maintained guides and decisions in committed docs. After implementation, fold lasting rules into their owning guide/ADR and remove completed plans, one-off research, logs and reports from the active tree. Preserve originals under ignored `docs/.local/` before removal or substantial rewriting; earlier committed versions remain in Git. Never alter frozen fixtures/receipts or legal notices to simplify prose.
- Production MCP JSON definitions belong in `crates/hiero/resources/mcp/`; active fixtures belong beside the owning crate’s tests. Do not restore broad compatibility/qualification trees or unused rewrite harnesses. Cache acquired assets under ignored `target/acquired-assets/`; optional desktop evidence uses `evidence/desktop/`.
- Markdown under `crates/hiero/resources/` is embedded agent-facing runtime input. Treat edits as product changes, not cosmetic documentation cleanup. Keep upstream/license provenance beside the component it describes. Local `.remember/` notes and `.agents/` skills are not public product guides.
- Check local links, heading anchors, documented commands and stale references. Use focused documentation/script checks for documentation-only changes; do not rebuild unchanged native binaries or add a documentation gate merely for file presence. Keep audit inventories and cleanup reports local rather than committing another maintenance document.

## Overengineering stance

Make sure to not overengineer checks. All the harnesses should be simple, robust, and not covered by overengineered tests. Keep it simple, stupid. Tests should cover business logic and edge cases, not just tests for the sake of tests. TDD is useful, but do not use it for every simple thing unless you covering a bug / feature.

## Simplicity and useful verification

- Tests must verify product behavior, business logic, or a real edge case. Remove tests that merely mirror implementation details, private helper structure, or source spelling.
- File-presence checks must serve startup, artifact integrity, data safety, or required distribution behavior. Do not gate runtime or publication on optional documentation, diagnostics, or a fixed inventory of licenses.
- Avoid duplicate handwritten inventories. Discover ancillary package documents during packaging; authenticate downloaded archives and validate safe extraction and critical executable/model/runtime inputs.
- Recoverable ancillary workflow failures must remain warnings and must not roll back working artifacts or user data. Preserve and report diagnostics; do not mark failed checks as passed.
- Favor a simple author-facing tool and a small development/release workflow over additional qualification layers.
