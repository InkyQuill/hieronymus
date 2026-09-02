# Frontend Embedding Qualification Record

## Decision

| Field | Value |
| --- | --- |
| Risk | frontend-embedding |
| Status | pass |
| Decision | qualified |
| Target | x86_64-unknown-linux-gnu |
| Acceptance owner | Pavel Obruchnikov &lt;me@inkyquill.net&gt; |

## Normative Specifications

- docs/adr/0014-web-console-replaces-terminal-ui.md
- docs/superpowers/specs/2026-08-31-rust-migration-program-design.md

## Replay Commands

| Order | Command |
| --- | --- |
| 1 | HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_frontend --write |
| 2 | unshare --user --map-root-user --net -- bun run --cwd frontend build -- --outDir ../qualification/.artifacts/frontend-dist/current --emptyOutDir |
| 3 | CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 build --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --target x86_64-unknown-linux-gnu |
| 4 | CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --check |
| 5 | CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/frontend-embedding CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/frontend-embedding/Cargo.toml --release --locked --all-targets --target x86_64-unknown-linux-gnu -- -D warnings |
| 6 | uv run python -m tools.qualification.validate qualification/records/frontend-embedding.json |
| 7 | uv run python -m tools.qualification.render --check qualification/records/frontend-embedding.json docs/qualification/rust/frontend-embedding.md |

## Environment

| Field | Value |
| --- | --- |
| Architecture | x86_64 |
| Bun | 1.4.0 |
| Cargo | cargo 1.96.0 &#40;30a34c682 2026-05-25&#41; |
| Kernel | 7.2.2-1-cachyos |
| Native libraries | ld-linux-x86-64.so.2, libc.so.6, libgcc_s.so.1, linux-vdso.so.1 |
| Operating system | linux |
| Rust compiler | rustc 1.96.0 &#40;ac68faa20 2026-05-25&#41; |
| Target | x86_64-unknown-linux-gnu |

## Locked Dependencies

| Name | Version | Source | Checksum | Features |
| --- | --- | --- | --- | --- |
| anyhow | 1.0.104 | registry+https://github.com/rust-lang/crates.io-index | 330a5ed07fa54e4702c9d6c4174f74427fc0ef6e214bbd677ae50a5099946470 | &#40;none&#41; |
| bitflags | 2.13.1 | registry+https://github.com/rust-lang/crates.io-index | b588b76d00fde79687d7646a9b5bdf3cc0f655e0bbd080335a95d7e96f3587da | &#40;none&#41; |
| block-buffer | 0.10.4 | registry+https://github.com/rust-lang/crates.io-index | 3078c7629b62d3f0439517fa394996acacc5cbc91c5a20d8c658e77abd503a71 | &#40;none&#41; |
| block-buffer | 0.12.1 | registry+https://github.com/rust-lang/crates.io-index | d2f6c7dbe95a6ed67ad9f18e57daf93a2f034c524b99fd2b76d18fdfeb6660aa | &#40;none&#41; |
| cfg-if | 1.0.4 | registry+https://github.com/rust-lang/crates.io-index | 9330f8b2ff13f34540b44e946ef35111825727b38d33286ef986142615121801 | &#40;none&#41; |
| const-oid | 0.10.2 | registry+https://github.com/rust-lang/crates.io-index | a6ef517f0926dd24a1582492c791b6a4818a4d94e789a334894aa15b0d12f55c | &#40;none&#41; |
| cpufeatures | 0.2.17 | registry+https://github.com/rust-lang/crates.io-index | 59ed5838eebb26a2bb2e58f6d5b5316989ae9d08bab10e0e6d103e656d1b0280 | &#40;none&#41; |
| cpufeatures | 0.3.1 | registry+https://github.com/rust-lang/crates.io-index | 5ca28b0ae3115b884660db4118d803791fd6756b6e88f39c0f3f7859060d7566 | &#40;none&#41; |
| crypto-common | 0.1.7 | registry+https://github.com/rust-lang/crates.io-index | 78c8292055d1c1df0cce5d180393dc8cce0abec0a7102adb6c7b1eef6016d60a | &#40;none&#41; |
| crypto-common | 0.2.2 | registry+https://github.com/rust-lang/crates.io-index | ce6e4c961d6cd6c9a86db418387425e8bdeaf05b3c8bc1411e6dca4c252f1453 | &#40;none&#41; |
| digest | 0.10.7 | registry+https://github.com/rust-lang/crates.io-index | 9ed9a281f7bc9b7576e61468ba615a66a5c8cfdff42420a70aa82701a3b1e292 | &#40;none&#41; |
| digest | 0.11.3 | registry+https://github.com/rust-lang/crates.io-index | f1dd6dbb5841937940781866fa1281a1ff7bd3bf827091440879f9994983d5c2 | &#40;none&#41; |
| dirs | 6.0.0 | registry+https://github.com/rust-lang/crates.io-index | c3e8aa94d75141228480295a7d0e7feb620b1a5ad9f12bc40be62411e38cce4e | &#40;none&#41; |
| dirs-sys | 0.5.0 | registry+https://github.com/rust-lang/crates.io-index | e01a3366d27ee9890022452ee61b2b63a67e6f13f58900b651ff5665f0bb1fab | &#40;none&#41; |
| errno | 0.3.14 | registry+https://github.com/rust-lang/crates.io-index | 39cab71617ae0d63f51a36d69f866391735b51691dbda63cf6f96d042b63efeb | &#40;none&#41; |
| fastrand | 2.5.0 | registry+https://github.com/rust-lang/crates.io-index | da7c62ceae207dd37ea5b845da6a0696c799f85e97da1ab5b7910be3c1c80223 | &#40;none&#41; |
| generic-array | 0.14.7 | registry+https://github.com/rust-lang/crates.io-index | 85649ca51fd72272d7821adaf274ad91c288277713d9c18820d8499a7ff69e9a | &#40;none&#41; |
| getrandom | 0.2.17 | registry+https://github.com/rust-lang/crates.io-index | ff2abc00be7fca6ebc474524697ae276ad847ad0a6b3faa4bcb027e9a4614ad0 | &#40;none&#41; |
| getrandom | 0.4.3 | registry+https://github.com/rust-lang/crates.io-index | 300e883d756b2e4ec94e02791f39b04b522276138852cfc41d9fb7e904106099 | &#40;none&#41; |
| hybrid-array | 0.4.14 | registry+https://github.com/rust-lang/crates.io-index | 707114b52a152fa7bdb290cd7cd5912d9467273b6d74e21b8d81aca1f8533f6b | &#40;none&#41; |
| itoa | 1.0.18 | registry+https://github.com/rust-lang/crates.io-index | 8f42a60cbdf9a97f5d2305f08a87dc4e09308d1276d28c869c684d7777685682 | &#40;none&#41; |
| libc | 0.2.189 | registry+https://github.com/rust-lang/crates.io-index | 3eaf3ede3fee6db1a4c2ee091bf8a8b4dccdc6d17f656fb07896ee72867612f2 | &#40;none&#41; |
| libredox | 0.1.23 | registry+https://github.com/rust-lang/crates.io-index | 8d8f1ea3f21fd3405dcaf6c9b5c1630af9afc422d9073ea39c5f6d6c772e08ed | &#40;none&#41; |
| linux-raw-sys | 0.12.1 | registry+https://github.com/rust-lang/crates.io-index | 32a66949e030da00e8c7d4434b251670a91556f4144941d37452769c25d58a53 | &#40;none&#41; |
| memchr | 2.8.3 | registry+https://github.com/rust-lang/crates.io-index | cf8baf1c55e62ffcace7a9f06f4bd9cd3f0c4beb022d3b367256b91b87513d98 | &#40;none&#41; |
| mime | 0.3.17 | registry+https://github.com/rust-lang/crates.io-index | 6877bb514081ee2a7ff5ef9de3281f14a4dd4bceac4c09388074a6b5df8a139a | &#40;none&#41; |
| mime_guess | 2.0.5 | registry+https://github.com/rust-lang/crates.io-index | f7c44f8e672c00fe5308fa235f821cb4198414e1c77935c1ab6948d3fd78550e | &#40;none&#41; |
| once_cell | 1.21.4 | registry+https://github.com/rust-lang/crates.io-index | 9f7c3e4beb33f85d45ae3e3a1792185706c8e16d043238c593331cc7cd313b50 | &#40;none&#41; |
| option-ext | 0.2.0 | registry+https://github.com/rust-lang/crates.io-index | 04744f49eae99ab78e0d5c0b603ab218f515ea8cfe5a456d7629ad883a3b6e7d | &#40;none&#41; |
| percent-encoding | 2.3.2 | registry+https://github.com/rust-lang/crates.io-index | 9b4f627cb1b25917193a259e49bdad08f671f8d9708acfd5fe0a8c1455d87220 | &#40;none&#41; |
| proc-macro2 | 1.0.107 | registry+https://github.com/rust-lang/crates.io-index | 985e7ec9bb745e6ce6535b544d84d6cd6f7ad8bd711c398938ae983b91a766d9 | &#40;none&#41; |
| quote | 1.0.47 | registry+https://github.com/rust-lang/crates.io-index | 1fbf4db142a473a8d80c26bbf18454ed458bf8d26c8219c331daecfdbd079001 | &#40;none&#41; |
| r-efi | 6.0.0 | registry+https://github.com/rust-lang/crates.io-index | f8dcc9c7d52a811697d2151c701e0d08956f92b0e24136cf4cf27b57a6a0d9bf | &#40;none&#41; |
| redox_users | 0.5.2 | registry+https://github.com/rust-lang/crates.io-index | a4e608c6638b9c18977b00b475ac1f28d14e84b27d8d42f70e0bf1e3dec127ac | &#40;none&#41; |
| rust-embed | 8.12.0 | registry+https://github.com/rust-lang/crates.io-index | e9e7760e252aaba7b09f4be00e36476cf585bdb68a53552ac954cdf504ab4bc9 | &#40;none&#41; |
| rust-embed-impl | 8.12.0 | registry+https://github.com/rust-lang/crates.io-index | 3bcfc4d6f53af43755f7a723e4b6b8794fcce052a178dd8c6c1dadc5f5343097 | &#40;none&#41; |
| rust-embed-utils | 8.12.0 | registry+https://github.com/rust-lang/crates.io-index | 42ffa149f6aa81b58a5b3011d01a857c4ed12c7a732d2c51947a4c7c692185f0 | &#40;none&#41; |
| rustix | 1.1.4 | registry+https://github.com/rust-lang/crates.io-index | b6fe4565b9518b83ef4f91bb47ce29620ca828bd32cb7e408f0062e9930ba190 | &#40;none&#41; |
| same-file | 1.0.6 | registry+https://github.com/rust-lang/crates.io-index | 93fc1dc3aaa9bfed95e02e6eadabb4baf7e3078b0bd1b4d7b6b0b68378900502 | &#40;none&#41; |
| serde | 1.0.229 | registry+https://github.com/rust-lang/crates.io-index | 4148590afebada386688f18773da617792bf2ef03ffc1e4cbd2b1d45b023e0ba | &#40;none&#41; |
| serde_core | 1.0.229 | registry+https://github.com/rust-lang/crates.io-index | 67dca2c9c51e58a4791a4b1ed58308b39c64224d349a935ab5039aa360942a48 | &#40;none&#41; |
| serde_derive | 1.0.229 | registry+https://github.com/rust-lang/crates.io-index | e7a5d71263a5a7d47b41f6b3f06ba276f10cc18b0931f1799f710578e2309348 | &#40;none&#41; |
| serde_json | 1.0.151 | registry+https://github.com/rust-lang/crates.io-index | c841b55ecdae098c80dcae9cf767f6f8a0c2cdb3416bbef72181df4d0fe73f14 | &#40;none&#41; |
| sha2 | 0.10.9 | registry+https://github.com/rust-lang/crates.io-index | a7507d819769d01a365ab707794a4084392c824f54a7a6a7862f8c3d0892b283 | &#40;none&#41; |
| sha2 | 0.11.0 | registry+https://github.com/rust-lang/crates.io-index | 446ba717509524cb3f22f17ecc096f10f4822d76ab5c0b9822c5f9c284e825f4 | &#40;none&#41; |
| shellexpand | 3.1.2 | registry+https://github.com/rust-lang/crates.io-index | 32824fab5e16e6c4d86dc1ba84489390419a39f97699852b66480bb87d297ed8 | &#40;none&#41; |
| syn | 2.0.119 | registry+https://github.com/rust-lang/crates.io-index | 872831b642d1a07999a962a351ed35b955ea2cfc8f3862091e2a240a84f17297 | &#40;none&#41; |
| syn | 3.0.4 | registry+https://github.com/rust-lang/crates.io-index | e6275cddf4610d1775e6d1fe9469b2e77d0f39fd98fb7450901b821e0c53649f | &#40;none&#41; |
| tempfile | 3.27.0 | registry+https://github.com/rust-lang/crates.io-index | 32497e9a4c7b38532efcdebeef879707aa9f794296a4f0244f6f69e9bc8574bd | &#40;none&#41; |
| thiserror | 2.0.20 | registry+https://github.com/rust-lang/crates.io-index | ec86235f5fcc2a73650310756d2ac5b138a5780bbbdfae3eeccec992c435ba4f | &#40;none&#41; |
| thiserror-impl | 2.0.20 | registry+https://github.com/rust-lang/crates.io-index | bc04cd3e1236dd4a98afca4569f2deb3f120e5422a4023be2cb683f8486292af | &#40;none&#41; |
| typenum | 1.20.1 | registry+https://github.com/rust-lang/crates.io-index | b6f5e870be6c3b371b77fe0ee0bafb859fa4964b4404c27de1d380043c4dda20 | &#40;none&#41; |
| unicase | 2.9.0 | registry+https://github.com/rust-lang/crates.io-index | dbc4bc3a9f746d862c45cb89d705aa10f187bb96c76001afab07a0d35ce60142 | &#40;none&#41; |
| unicode-ident | 1.0.24 | registry+https://github.com/rust-lang/crates.io-index | e6e4313cd5fcd3dad5cafa179702e2b244f760991f45397d14d4ebf38247da75 | &#40;none&#41; |
| version_check | 0.9.5 | registry+https://github.com/rust-lang/crates.io-index | 0b928f33d975fc6ad9f86c8f283853ad26bdd5b10b7f1542aa2fa15e2289105a | &#40;none&#41; |
| walkdir | 2.5.0 | registry+https://github.com/rust-lang/crates.io-index | 29790946404f91d9c5d06f9874efddea1dc06c5efe94541a7d6863108e3a5e4b | &#40;none&#41; |
| wasi | 0.11.1+wasi-snapshot-preview1 | registry+https://github.com/rust-lang/crates.io-index | ccf3ec651a847eb01de73ccad15eb7d99f80485de043efb2f370cd654f4ea44b | &#40;none&#41; |
| winapi-util | 0.1.11 | registry+https://github.com/rust-lang/crates.io-index | c2a7b1c03c876122aa43f3020e6c3c3ee5c05081c9a00739faf7503aeba10d22 | &#40;none&#41; |
| windows-link | 0.2.1 | registry+https://github.com/rust-lang/crates.io-index | f0805222e57f7521d6a62e36fa9163bc891acd422f971defe97d64e70d0a4fe5 | &#40;none&#41; |
| windows-sys | 0.61.2 | registry+https://github.com/rust-lang/crates.io-index | ae137229bcbd6cdf0f7b80a31df61766145077ddf49416a728b02cb3921ff3fc | &#40;none&#41; |
| zmij | 1.0.23 | registry+https://github.com/rust-lang/crates.io-index | 29666d0abbfad1e3dc4dcf6144730dd3a3ab225bbbdac83319345b1b44ccfc1b | &#40;none&#41; |

## Required Criteria

| Criterion | Status | Summary | Measurements | Not-run reason |
| --- | --- | --- | --- | --- |
| bun-version-and-frozen-build | pass | bun matched the pinned prerequisite, the network-isolated frozen-lockfile bundle build, locked release build, and clippy gate all succeeded | {"build_output_sha256":"07789def5c6f44eaab13846774e3282aef52fdd78bcd0c03c7f7677c8ab9768f","bun_version":"1.4.0","bundle_bytes":309723,"bundle_files":6,"bundle_sha256":"c323573dd067b069b88c7984320dac811593c15e1727c3e051c8a71607a9717c","clippy_output_sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","lock_sha256":"b9a58a6e17accb67a50c5f2cca53e79ce7d36e3cd0d3b14a3c501aa4eb878471"} | &#40;none&#41; |
| missing-bundle-rejected | pass | with the canonical bundle renamed away, a fresh bounded cargo target failed to compile at the build-script guard and was removed | {"missing_target_removed":1,"rejected_builds":1,"rejected_exit_code":101} | &#40;none&#41; |
| release-assets-embedded | pass | the embedded manifest's relative files, byte lengths, and SHA-256 values equal the copied Vite output exactly | {"byte_identity":1,"embedded_files":6,"manifest_files":6} | &#40;none&#41; |
| index-and-spa-fallback | pass | the embedded root served the real index document and every admin/config route fell back to it with the fixture HTML type | {"fallback_routes":4,"index_status":200,"replayed_routes":5} | &#40;none&#41; |
| hashed-asset-and-mime | pass | every hashed bundle asset was served by digest and byte length with its extension MIME type, including the fixture's JavaScript assets route | {"assets":5,"fixture_route":"frontend.route.get.assets.path","javascript_assets":1} | &#40;none&#41; |
| missing-asset-404 | pass | missing paths beneath assets returned the empty no-fallback JSON 404 shape for the fixture failure path and a qualification probe | {"cases":2,"fallbacks":0,"status":404} | &#40;none&#41; |
| runtime-asset-root-inaccessible | pass | with the bundle renamed beneath an access-denied quarantine, every straced binary probe returned the baseline digest, referenced no asset-root path, and executed only the release binary | {"asset_root_references":0,"digests_matched":14,"permission_denied":1,"probes":14} | &#40;none&#41; |
| no-runtime-bun-node-python | pass | ldd basenames and the traced exec tree contain only libc-family libraries and the release binary itself | {"native_library_basenames":&#91;"ld-linux-x86-64.so.2","libc.so.6","libgcc_s.so.1","linux-vdso.so.1"&#93;,"runtime_processes":0,"traced_execs":14} | &#40;none&#41; |
| no-source-map-secret | pass | the embedded bundle contains no source maps and none of the forbidden secret markers appear in any embedded byte | {"files_scanned":6,"forbidden_markers":0,"scanned_bytes":309723,"source_maps":0} | &#40;none&#41; |
| binary-size-recorded | pass | the release binary byte size was recorded with no size threshold | {"binary_bytes":1212056,"bundle_bytes":309723,"embedded_files":6} | &#40;none&#41; |

## Consumed Compatibility Contracts

- frontend.route.get.admin
- frontend.route.get.admin.path
- frontend.route.get.assets.path
- frontend.route.get.config
- frontend.route.get.config.path
- frontend.route.get.root

## Input Fingerprint

SHA-256: `3924e2e79f2d3358b6119a86abe10f8f147ce5fd5e70b7d566ff11d5307e5306`

Input paths:

- qualification/prerequisites.json
- qualification/rust-toolchain.toml
- tools/qualification/model.py
- tools/qualification/fingerprint.py
- tools/qualification/projections.py
- tools/qualification/redaction.py
- tools/qualification/validate.py
- tools/qualification/render.py
- tools/qualification/acquire.py
- tools/qualification/process.py
- tools/qualification/clean.py
- tools/qualification/run_frontend.py
- qualification/harnesses/frontend-embedding/Cargo.toml
- qualification/harnesses/frontend-embedding/Cargo.lock
- qualification/harnesses/frontend-embedding/build.rs
- qualification/harnesses/frontend-embedding/src/main.rs
- qualification/harnesses/frontend-embedding/src/assets.rs
- qualification/harnesses/frontend-embedding/tests/assets.rs
- frontend/index.html
- frontend/package.json
- frontend/bun.lock
- frontend/tsconfig.json
- frontend/vite.config.ts
- frontend/src/web/App.svelte
- frontend/src/web/app.css
- frontend/src/web/app.test.ts
- frontend/src/web/components/AdminDashboard.svelte
- frontend/src/web/components/DreamingEditor.svelte
- frontend/src/web/components/IngestEditor.svelte
- frontend/src/web/components/MemoryViews.svelte
- frontend/src/web/components/MemoryViews.test.ts
- frontend/src/web/components/ProviderEditor.svelte
- frontend/src/web/components/ReleaseEditor.svelte
- frontend/src/web/components/Toast.svelte
- frontend/src/web/components/editors.test.ts
- frontend/src/web/fonts.css
- frontend/src/web/fonts/geist.woff2
- frontend/src/web/fonts/inconsolatalgc.woff2
- frontend/src/web/fonts/literata.woff2
- frontend/src/web/lib/admin-events.svelte.ts
- frontend/src/web/lib/api.ts
- frontend/src/web/lib/theme.svelte.test.ts
- frontend/src/web/lib/theme.svelte.ts
- frontend/src/web/lib/types.ts
- frontend/src/web/main.ts
- frontend/src/web/test/setup.ts
- compatibility/fixtures/http/route-cases.json

## Cleanup Assertions

| Field | Value |
| --- | --- |
| work_dir_removed | true |
| raw_logs_removed | true |
| install_dir_removed | true |
| source_inputs_unchanged | true |
| user_data_opened | false |
| core_dumps_disabled | true |
| owned_process_groups_reaped | true |

## Review

| Field | Value |
| --- | --- |
| Owner | Pavel Obruchnikov &lt;me@inkyquill.net&gt; |
| Status | pending |
| Objective evidence reviewed | false |
| Normative constraints preserved | false |

## Immutable Consequence

&#40;none&#41;
