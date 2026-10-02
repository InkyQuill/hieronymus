# PDF dependency simplification decision

Issue #132, inspected 2026-10-03 against the locked `pdf-extract 0.12.0` and `lopdf 0.42.0` sources and Cargo feature resolution.

**Keep the current extractor.** There is no unused default-feature bundle to disable: pdf-extract defines no features, and already depends on lopdf with `default-features = false`. Resolved lopdf enables only `wasm_js`; its optional chrono, jiff, rayon, time and image features are absent. Adding `default-features = false` to our pdf-extract declaration would remove zero dependencies and change no compilation units.

The full resolved hieronymus dependency graph has 29 package-version nodes reachable exclusively through pdf-extract (including the extractor itself). This is a resolution graph, not a claim that every target builds every node. They include PDF/font parsers, character encodings, compression and encrypted-document primitives. Removing them requires removing or replacing PDF extraction, not a feature toggle. No private fork is justified just to drop wasm support: the application targets native platforms and those platform-specific units are not built there.

A `pdftotext` subprocess could move parsing out of Cargo but would add an installed external tool and platform-specific acquisition/versioning. That makes the local desktop package harder to operate. A direct lopdf text extractor would retain most of the same parser tree while making us responsible for font/encoding extraction quality. Neither meets the current priority of simpler operation with reliable text. No replacement or new dependency is introduced.

Validation: the existing PDF import regression verifies extracted text is indexed and searchable; the empty/scanned document regression verifies explicit rejection instead of fabricated text. Both passed. No binary, database or build-time delta is claimed because production inputs did not change. A cold rebuild of identical inputs would measure noise, not an optimization.

Reproduce the feature audit with `cargo tree -p pdf-extract -e features` and inspect the locked registry manifests. Keep PDF handling bounded by the existing import path. Revisit only with a concrete representative corpus and a replacement that improves extraction or materially reduces deployment complexity.
