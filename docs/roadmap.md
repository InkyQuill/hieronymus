# Hieronymus Roadmap

Hieronymus is an alpha local-first memory system for writing and literary
translation. The main mechanics are implemented in Rust. Current work focuses on
optimization and fixes to runtime behavior, not a pending language rewrite.
Historical Python and port-planning documents are available in Git history.

## Product Direction

[ADR 0016](adr/0016-autonomous-story-memory-product-vision.md) supersedes
ADR 0005 and the human-only terminology lifecycle in ADR 0011. The target is
autonomous story memory, with user corrections instead of required review,
and agent plugins with shared workflow skills; Pi has a native package using the installed MCP adapter, while zCode's shared-Claude evidence is paused/unqualified
bundle. Its product principles and acceptance scenarios guide behavior fixes; older
implementation-gap lists must be checked against current Rust code. Historical documents do not require preserving human approval gates.

## Release backlog — 1.0

- [ ] **Retire LanceDB transition compatibility when preparing 1.0.** Deferred
  until the explicitly approved 1.0 release; keep the transition behavior during
  0.x. Audit and remove LanceDB-specific recovery assumptions, compatibility
  tests and obsolete upgrade documentation after the SQLite vector-store
  transition ([ADR 0013](adr/0013-semantic-index-and-platform-support.md)).
  Define the supported upgrade path for installations that still contain old
  LanceDB generations and document how their unused files can be cleaned up.
  Preserve authoritative SQLite data and generic missing/corrupt-index rebuild
  behavior; retiring compatibility must not silently delete user data or bring
  back LanceDB dependencies. Historical research evidence may remain archived.

## Current Rust implementation

The core Rust implementation covers configuration and storage, series and memory
lifecycle, deterministic terminology, recall and RAG, Dreaming, daemon and MCP
transports, the Svelte console, migration tooling and distribution. Implemented
mechanics still need behavioral fixes and optimization; implementation does not
imply complete real-model or cross-platform qualification.

Semantic retrieval uses bundled SQLite and exact cosine ranking in Rust.
The model, ONNX Runtime and tokenizer remain unchanged. See the
[SQLite decision and validation](https://github.com/InkyQuill/hieronymus/blob/87a74f9029389ad670a950468aa66488554f7c47/docs/research/2026-10-02-sqlite-vector-decision.md).

## Active work — optimization and behavior fixes

- Fix memory, recall and Dreaming behavior against current product principles,
  with focused regression tests and preservation of authoritative user data.
- Reduce build dependencies and repeated compilation while preserving retrieval
  accuracy and import quality. Measure actual build and artifact deltas rather
  than treating overlapping dependency subtrees as removable package counts.
- Use SVG icons from an existing icon package. Retire the custom icon generator;
  choose assets and platform delivery that preserve tray states, themes and sizes
  without introducing another heavy runtime rendering stack.
- Investigate unnecessary PDF-import features with extraction-quality fixtures.
- Keep TypeSafe SDK as a dependency; SDK replacement or reduction is not part of
  the current optimization scope.
- Investigate model, ONNX and tokenizer footprint separately after storage/build
  simplification; keep the inference stack unchanged for this phase.

The earlier Rust-port slice plan is historical. Follow current amended ADRs and
verified runtime behavior instead of its obsolete pre-cutover gap list.
