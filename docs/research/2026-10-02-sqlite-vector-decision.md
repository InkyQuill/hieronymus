# Exact SQLite vector storage

The user prioritizes fewer dependencies, a simple implementation, and exact
retrieval over storage size and submillisecond latency. Choose the existing
rusqlite dependency and exact cosine search in Rust; defer sqlite-vec until a
measured need justifies another native component. No inference/model changes.

An isolated storage experiment at commit 013a573 compared plain SQLite, sqlite-vec,
USearch F32/F16/global/per-series, and the unchanged Lance module. On normalized
synthetic 384d vectors exact backends returned 100% recall@10. Lance IVF-PQ at the
current 16 partitions/16 probes/refine 8 returned 28.67–30.67% for the largest tenant
of the 100k clustered fixture across three builds; this is not a real-book quality
claim. Release compilation was 26s for SQLite versus 929s for the isolated Lance
probe; these are not full-application timings. Experimental sources and raw data
remain in the separate codex/vector-store-research worktree.

Implementation preserves the existing generation/job/corpus-revision protocol,
uses one disposable SQLite file per generation and a separate artifact namespace,
validates complete model identity, and keeps old Lance artifacts intact. This is
a derived-index rebuild on upgrade, not a main-database migration. See ADR 0013's
2026-10-02 amendment. No deployment, install, release or user-data cleanup occurs
as part of this change.

Runtime-path clarification: at the baseline commit `create_ann_index` is called
only by tests, not by the production generation builder. The normal existing
backend therefore corresponds to the exact Lance flat benchmark (100% recall),
not the additional IVF-PQ benchmark. Low IVF-PQ recall is not evidence that the
ordinary installed runtime was losing neighbors. Dependency reduction and simple
exact semantics are the reasons for replacing the production backend.

Verification on 2026-10-02: cargo fmt, Clippy (all targets/features, locked, warnings
as errors), 1696 Rust tests passed with 20 explicitly ignored, rustdoc (warnings as
errors), 158 script tests, frontend typecheck, 113 frontend tests and production
frontend build passed. Real-model and native installed-artifact/platform checks
were not run. Static review found no remaining blockers; full-generation scanning
on serving reads remains the documented performance tradeoff.

The workspace lockfile fell from 676 to 384 packages: 292 removed, zero added. The
normal application dependency tree contains no Lance, Arrow, DataFusion or
Protobuf packages. Application CI no longer installs protoc. Source changes and
tests are local to codex/sqlite-semantic-index; no installation or release was
performed. Logs for this local run: /tmp/hieronymus-sqlite-verification/.

## Follow-up dependency audit (draft PR backlog)

Local Linux normal dependency subtrees (`cargo tree -e normal`, unique package
name/version pairs, including the root): hiero 217, hiero-desktop 293,
pdf-extract 61, resvg 34, typesafe-sdk-rust 32, tokenizers 72, rustls 12.
Subtrees overlap; these counts are not exclusive removable dependencies and
exclude build/dev edges. The 384-entry lockfile also covers other targets.

- [ ] Measure PDF import feature reduction first. A current debug lopdf rlib is
  about 19 MiB. Inspect enabled features and compare extraction against existing
  PDF fixtures before considering a replacement or optional import component.
- [ ] Use SVG assets from an existing icon package and remove the custom icon
  generator. Validate tray states, themes, sizes and native platform delivery;
  avoid replacing resvg with another heavy runtime renderer.
- TypeSafe SDK remains an accepted dependency, outside current optimization scope
  (owner decision, 2026-10-02).
- [ ] Measure duplicate test executables and compilation units separately from
  third-party packages. This verification target/debug occupies about 4.1 GiB;
  individual rlibs and accumulated targets do not establish installed binary size.

Tokenizers (about 27 MiB per current debug rlib), ort and model assets remain a
separate future inference investigation. Keep the existing rustls security
behavior; its roughly 13 MiB debug rlib alone is not justification for replacement.
For each candidate, record isolated cold/warm builds, artifact count/bytes and
stripped release binary delta, with unchanged functionality and quality fixtures.
No further dependency removals are implemented by this preliminary audit.
