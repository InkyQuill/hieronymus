# Native platform artifacts

This reference owns native artifact layout and integrity. Actual desktop/session
acceptance is separate; use [desktop checks](desktop-qualification.md).

## Targets and pins

| Target | Runtime acquisition |
| --- | --- |
| `x86_64-unknown-linux-gnu` | Pinned official Linux archive |
| `x86_64-pc-windows-msvc` | Pinned official archive and companion DLL |
| `aarch64-apple-darwin` | Pinned official Apple Silicon archive |
| `x86_64-apple-darwin` | Reviewed source-built archive with source/run/receipt provenance |

The authoritative archive/member hashes, sizes and source provenance are in
[`onnxruntime-targets.json`](../scripts/onnxruntime-targets.json). Intel's reviewed
source route is already pinned; native desktop use remains unqualified. The measured
macOS runtime load-command floor is 14.0, which is not a whole-app compatibility
claim. Windows runtime/CRT deployment belongs to native installer checks.

Model/tokenizer identity is pinned separately from native runtime. It remains
`paraphrase-multilingual-MiniLM-L12-v2` at revision
`e8f8c211226b894fcb81acc59f3b34ba3efd5f42`, with ONNX Runtime 1.28.0.
Acquisition verification is not installed inference or interactive acceptance.

## Common model and target metadata

[`scripts/shared-model.ts`](../scripts/shared-model.ts) produces canonical regular
ustar members in fixed order/metadata with pinned Bun. Produce once and distribute
identical archive bytes plus `ModelArtifact` metadata to all target jobs. Existing
output must match its digest; never overwrite an immutable artifact.

Format-2 `release-<triple>.json` binds application version, exact target/channel,
platform archive/digest and common model name/revision/archive/digest/member map,
with the current `signature:null` policy. Windows uses ZIP, others tar.gz.
Unknown/duplicate fields, unsupported signatures, model drift and wrong targets
are refused. Legacy monolithic input uses a separate decoder; split output has
no `release.json` alias. HTTPS acquisition falls back to legacy only on explicit
404, never on malformed metadata, TLS failure or a missing model artifact.

The platform archive carries executables/helper/launcher as applicable, `lib/`,
notices and `assets.json`, with no model payload. The model archive carries the
pinned `models/minilm` files, with no runtime members. Installation assembles
verified copies into each immutable version; a rehashed content-addressed cache
is acquisition only, never runtime authority.

## Verification and extraction

Authenticate both archive digests into owned retained snapshots before inspecting
or extracting members. Subsequent source-file edits cannot change accepted bytes.
Snapshot storage is bounded, privately created and released on every path;
no source directory is modified. Combined extraction remains bounded.

Reject traversal, duplicate/colliding/undeclared members, missing pins, unsafe links,
devices and expansion overflow. A failed fresh assembly removes only its own output
and reports cleanup failure. Unix aliases are restricted to literal links to `hiero`.
Windows framing validates strict ZIP32 local/central identity before the archive
library can coalesce names: no encryption, ZIP64, descriptors, overlap or extras.

`assets.json` binds final platform members except itself and all model members.
Produce it after stripping/applicable signing, never changing pinned upstream bytes.
macOS sealed app manifests/receipts remain outside the seal to avoid digest cycles.
A desktop install requires the matching helper even where a general headless verifier
allows its absence. Native filesystem paths preserve drive/UNC roots while validating
traversed components; portable path tests do not qualify Windows filesystem behavior.

`hiero release-verify --release-dir DIR [--output ABSENT_DIR]` uses the same typed
verification/assembly path. Inputs must remain immutable throughout packaging.

## Source-built Intel runtime

[`scripts/build-intel-runtime.ts`](../scripts/build-intel-runtime.ts) requires native
Intel macOS and the pinned upstream source. It records source/submodules, toolchain,
commands, measured dependencies/OS floor, tests and archive/member hashes. A candidate
receipt alone does not grant loading authority: reviewed pins do. Python is an
upstream ONNX source-build tool, not a Hieronymus runtime dependency.

Runtime source tests, final packaged inference and visible desktop workflows are
separate evidence. Record their gaps honestly rather than turning reviewed acquisition
into a compatibility or performance claim.
