# Legacy Database Import Qualification Record

## Decision

| Field | Value |
| --- | --- |
| Risk | legacy-database-import |
| Status | pass |
| Decision | qualified |
| Target | x86_64-unknown-linux-gnu |
| Acceptance owner | Pavel Obruchnikov &lt;me@inkyquill.net&gt; |

## Normative Specifications

- docs/adr/0010-data-locations-schema-ownership-and-upgrade.md
- docs/superpowers/specs/2026-08-31-rust-database-upgrade-design.md

## Replay Commands

| Order | Command |
| --- | --- |
| 1 | HIERONYMUS_QUALIFICATION_LIVE=1 CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true uv run python -m tools.qualification.run_database --write |
| 2 | CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 fmt --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --check |
| 3 | CARGO_TARGET_DIR=qualification/.artifacts/cargo-target/legacy-database-import CARGO_NET_OFFLINE=true cargo +1.96.0 clippy --manifest-path qualification/harnesses/legacy-database-import/Cargo.toml --locked --all-targets --target x86_64-unknown-linux-gnu -- -D warnings |
| 4 | uv run python -m tools.qualification.validate qualification/records/legacy-database-import.json |
| 5 | uv run python -m tools.qualification.render --check qualification/records/legacy-database-import.json docs/qualification/rust/legacy-database-import.md |

## Environment

| Field | Value |
| --- | --- |
| Architecture | x86_64 |
| Bun | &#40;none&#41; |
| Cargo | cargo 1.96.0 &#40;30a34c682 2026-05-25&#41; |
| Kernel | 7.2.2-1-cachyos |
| Native libraries | ld-linux-x86-64.so.2, libc.so.6, libgcc_s.so.1, libm.so.6, linux-vdso.so.1 |
| Operating system | linux |
| Rust compiler | rustc 1.96.0 &#40;ac68faa20 2026-05-25&#41; |
| Target | x86_64-unknown-linux-gnu |

## Locked Dependencies

| Name | Version | Source | Checksum | Features |
| --- | --- | --- | --- | --- |
| anyhow | 1.0.104 | registry+https://github.com/rust-lang/crates.io-index | 330a5ed07fa54e4702c9d6c4174f74427fc0ef6e214bbd677ae50a5099946470 | &#40;none&#41; |
| bitflags | 2.13.1 | registry+https://github.com/rust-lang/crates.io-index | b588b76d00fde79687d7646a9b5bdf3cc0f655e0bbd080335a95d7e96f3587da | &#40;none&#41; |
| block-buffer | 0.10.4 | registry+https://github.com/rust-lang/crates.io-index | 3078c7629b62d3f0439517fa394996acacc5cbc91c5a20d8c658e77abd503a71 | &#40;none&#41; |
| cc | 1.4.4 | registry+https://github.com/rust-lang/crates.io-index | 0ad534f4357a5264cce5019c989cf66a4f0dc4e0d1b1d15f8aacec0ff7360273 | &#40;none&#41; |
| cfg-if | 1.0.4 | registry+https://github.com/rust-lang/crates.io-index | 9330f8b2ff13f34540b44e946ef35111825727b38d33286ef986142615121801 | &#40;none&#41; |
| cpufeatures | 0.2.17 | registry+https://github.com/rust-lang/crates.io-index | 59ed5838eebb26a2bb2e58f6d5b5316989ae9d08bab10e0e6d103e656d1b0280 | &#40;none&#41; |
| crypto-common | 0.1.7 | registry+https://github.com/rust-lang/crates.io-index | 78c8292055d1c1df0cce5d180393dc8cce0abec0a7102adb6c7b1eef6016d60a | &#40;none&#41; |
| digest | 0.10.7 | registry+https://github.com/rust-lang/crates.io-index | 9ed9a281f7bc9b7576e61468ba615a66a5c8cfdff42420a70aa82701a3b1e292 | &#40;none&#41; |
| errno | 0.3.14 | registry+https://github.com/rust-lang/crates.io-index | 39cab71617ae0d63f51a36d69f866391735b51691dbda63cf6f96d042b63efeb | &#40;none&#41; |
| fallible-iterator | 0.3.0 | registry+https://github.com/rust-lang/crates.io-index | 2acce4a10f12dc2fb14a218589d4f1f62ef011b2d0cc4b3cb1bba8e94da14649 | &#40;none&#41; |
| fallible-streaming-iterator | 0.1.9 | registry+https://github.com/rust-lang/crates.io-index | 7360491ce676a36bf9bb3c56c1aa791658183a54d2744120f27285738d90465a | &#40;none&#41; |
| fastrand | 2.5.0 | registry+https://github.com/rust-lang/crates.io-index | da7c62ceae207dd37ea5b845da6a0696c799f85e97da1ab5b7910be3c1c80223 | &#40;none&#41; |
| find-msvc-tools | 0.1.11 | registry+https://github.com/rust-lang/crates.io-index | d45db016d36b838f563236e9193d0ee6ce38f3f68b6c94e914b4929c96bbb890 | &#40;none&#41; |
| generic-array | 0.14.7 | registry+https://github.com/rust-lang/crates.io-index | 85649ca51fd72272d7821adaf274ad91c288277713d9c18820d8499a7ff69e9a | &#40;none&#41; |
| getrandom | 0.4.3 | registry+https://github.com/rust-lang/crates.io-index | 300e883d756b2e4ec94e02791f39b04b522276138852cfc41d9fb7e904106099 | &#40;none&#41; |
| itoa | 1.0.18 | registry+https://github.com/rust-lang/crates.io-index | 8f42a60cbdf9a97f5d2305f08a87dc4e09308d1276d28c869c684d7777685682 | &#40;none&#41; |
| libc | 0.2.189 | registry+https://github.com/rust-lang/crates.io-index | 3eaf3ede3fee6db1a4c2ee091bf8a8b4dccdc6d17f656fb07896ee72867612f2 | &#40;none&#41; |
| libsqlite3-sys | 0.38.2 | registry+https://github.com/rust-lang/crates.io-index | f1d20bef17f513b9b3004532233187769cd072d790971f4e4da0e346eb6401e8 | &#40;none&#41; |
| linux-raw-sys | 0.12.1 | registry+https://github.com/rust-lang/crates.io-index | 32a66949e030da00e8c7d4434b251670a91556f4144941d37452769c25d58a53 | &#40;none&#41; |
| memchr | 2.8.3 | registry+https://github.com/rust-lang/crates.io-index | cf8baf1c55e62ffcace7a9f06f4bd9cd3f0c4beb022d3b367256b91b87513d98 | &#40;none&#41; |
| once_cell | 1.21.4 | registry+https://github.com/rust-lang/crates.io-index | 9f7c3e4beb33f85d45ae3e3a1792185706c8e16d043238c593331cc7cd313b50 | &#40;none&#41; |
| pkg-config | 0.3.34 | registry+https://github.com/rust-lang/crates.io-index | f6b464fbc74e149a392436b17d523f769e057cb6877f6a5c4618bc6f11800548 | &#40;none&#41; |
| proc-macro2 | 1.0.107 | registry+https://github.com/rust-lang/crates.io-index | 985e7ec9bb745e6ce6535b544d84d6cd6f7ad8bd711c398938ae983b91a766d9 | &#40;none&#41; |
| quote | 1.0.47 | registry+https://github.com/rust-lang/crates.io-index | 1fbf4db142a473a8d80c26bbf18454ed458bf8d26c8219c331daecfdbd079001 | &#40;none&#41; |
| r-efi | 6.0.0 | registry+https://github.com/rust-lang/crates.io-index | f8dcc9c7d52a811697d2151c701e0d08956f92b0e24136cf4cf27b57a6a0d9bf | &#40;none&#41; |
| rusqlite | 0.40.2 | registry+https://github.com/rust-lang/crates.io-index | 23f2a97da3e3873c73cb2a2e71b35c40ff95e0b1eefa8d72d8499a6928c3b5b3 | &#40;none&#41; |
| rustix | 1.1.4 | registry+https://github.com/rust-lang/crates.io-index | b6fe4565b9518b83ef4f91bb47ce29620ca828bd32cb7e408f0062e9930ba190 | &#40;none&#41; |
| serde | 1.0.229 | registry+https://github.com/rust-lang/crates.io-index | 4148590afebada386688f18773da617792bf2ef03ffc1e4cbd2b1d45b023e0ba | &#40;none&#41; |
| serde_core | 1.0.229 | registry+https://github.com/rust-lang/crates.io-index | 67dca2c9c51e58a4791a4b1ed58308b39c64224d349a935ab5039aa360942a48 | &#40;none&#41; |
| serde_derive | 1.0.229 | registry+https://github.com/rust-lang/crates.io-index | e7a5d71263a5a7d47b41f6b3f06ba276f10cc18b0931f1799f710578e2309348 | &#40;none&#41; |
| serde_json | 1.0.151 | registry+https://github.com/rust-lang/crates.io-index | c841b55ecdae098c80dcae9cf767f6f8a0c2cdb3416bbef72181df4d0fe73f14 | &#40;none&#41; |
| sha2 | 0.10.9 | registry+https://github.com/rust-lang/crates.io-index | a7507d819769d01a365ab707794a4084392c824f54a7a6a7862f8c3d0892b283 | &#40;none&#41; |
| shlex | 2.0.1 | registry+https://github.com/rust-lang/crates.io-index | f8fadd59c855ef2080decdef8ff161eb6661b86933c9d82e5ba29dc602a55aba | &#40;none&#41; |
| smallvec | 1.16.0 | registry+https://github.com/rust-lang/crates.io-index | b9be42f50aa861c555654aa3a37f52f4b1074bacf4e48fe0ef7fa584e80f1f0f | &#40;none&#41; |
| syn | 3.0.4 | registry+https://github.com/rust-lang/crates.io-index | e6275cddf4610d1775e6d1fe9469b2e77d0f39fd98fb7450901b821e0c53649f | &#40;none&#41; |
| tempfile | 3.27.0 | registry+https://github.com/rust-lang/crates.io-index | 32497e9a4c7b38532efcdebeef879707aa9f794296a4f0244f6f69e9bc8574bd | &#40;none&#41; |
| typenum | 1.20.1 | registry+https://github.com/rust-lang/crates.io-index | b6f5e870be6c3b371b77fe0ee0bafb859fa4964b4404c27de1d380043c4dda20 | &#40;none&#41; |
| unicode-ident | 1.0.24 | registry+https://github.com/rust-lang/crates.io-index | e6e4313cd5fcd3dad5cafa179702e2b244f760991f45397d14d4ebf38247da75 | &#40;none&#41; |
| vcpkg | 0.2.15 | registry+https://github.com/rust-lang/crates.io-index | accd4ea62f7bb7a82fe23066fb0957d48ef677f6eeb8215f372f52e48bb32426 | &#40;none&#41; |
| version_check | 0.9.5 | registry+https://github.com/rust-lang/crates.io-index | 0b928f33d975fc6ad9f86c8f283853ad26bdd5b10b7f1542aa2fa15e2289105a | &#40;none&#41; |
| windows-link | 0.2.1 | registry+https://github.com/rust-lang/crates.io-index | f0805222e57f7521d6a62e36fa9163bc891acd422f971defe97d64e70d0a4fe5 | &#40;none&#41; |
| windows-sys | 0.61.2 | registry+https://github.com/rust-lang/crates.io-index | ae137229bcbd6cdf0f7b80a31df61766145077ddf49416a728b02cb3921ff3fc | &#40;none&#41; |
| zmij | 1.0.23 | registry+https://github.com/rust-lang/crates.io-index | 29666d0abbfad1e3dc4dcf6144730dd3a3ab225bbbdac83319345b1b44ccfc1b | &#40;none&#41; |

## Required Criteria

| Criterion | Status | Summary | Measurements | Not-run reason |
| --- | --- | --- | --- | --- |
| bundled-sqlite-fts5 | pass | the locked release build over the bundled-SQLite rusqlite candidate succeeded and links no external sqlite library | {"binary_bytes":3603624,"external_sqlite_libraries":0,"locked_commands":1,"native_library_basenames":&#91;"ld-linux-x86-64.so.2","libc.so.6","libgcc_s.so.1","libm.so.6","linux-vdso.so.1"&#93;,"output_sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","toolchain":"1.96.0"} | &#40;none&#41; |
| fixture-classification | pass | all six frozen fixtures classified read-only exactly as the verified projection's variants expect | {"classifications":&#91;"minimal-python.sqlite=supported-python","legacy-python.sqlite=supported-legacy-python","empty.sqlite=empty","partial-python.sqlite=partial-python","corrupt.sqlite=corrupt","unknown-schema.sqlite=unknown-schema"&#93;,"fixtures":6,"matched":6} | &#40;none&#41; |
| supported-current-read | pass | the full current-schema fixture imported into a disposable neutral target with byte-identical frozen sources | {"probe_row_count":57,"source_bytes_identical":1,"source_name":"minimal-python.sqlite"} | &#40;none&#41; |
| supported-legacy-read | pass | the legacy-schema fixture imported into a disposable neutral target with byte-identical frozen sources | {"probe_row_count":1,"source_bytes_identical":1,"source_name":"legacy-python.sqlite"} | &#40;none&#41; |
| typed-row-accounting | pass | every source row of both supported fixtures produced exactly one neutral probe row and one ledger outcome | {"probe_rows":58,"probes":2} | &#40;none&#41; |
| fts-query-equivalence | pass | all five projected FTS probes matched their representative-row terms with stable matched-id digests | {"matched_probes":5,"probes":5} | &#40;none&#41; |
| ledger-preserved | pass | the neutral probe ledger recorded a clean read outcome for every row with nothing skipped or blocking | {"blocking":0,"ledger_read":58,"ledgers":2,"skipped":0} | &#40;none&#41; |
| unsupported-fail-closed | pass | every non-convertible fixture was refused before any target was created, each with its exact error code | {"error_codes":&#91;"empty.sqlite=empty-database","partial-python.sqlite=partial-schema","corrupt.sqlite=source-unreadable","unknown-schema.sqlite=unknown-schema"&#93;,"refusals":4} | &#40;none&#41; |
| source-byte-identity | pass | all six frozen fixture digests and the fixture tree digest are byte-identical before and after every child | {"fixtures":6,"probe_reports":2,"unchanged":6} | &#40;none&#41; |
| no-sensitive-row-output | pass | every captured harness output matched its exact digest-only schema and none carries a forbidden marker | {"captures":12,"forbidden_markers":0,"schema_valid":12} | &#40;none&#41; |

## Consumed Compatibility Contracts

- database.migrations.current
- database.schema.current
- database.upgrade.preflight

## Input Fingerprint

SHA-256: `ef77ae676474a45bbe46ff065f6e987ce74caf7f7f4ce249eb95364af2a3ccb5`

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
- tools/qualification/run_database.py
- qualification/harnesses/legacy-database-import/Cargo.toml
- qualification/harnesses/legacy-database-import/Cargo.lock
- qualification/harnesses/legacy-database-import/src/main.rs
- qualification/harnesses/legacy-database-import/src/classify.rs
- qualification/harnesses/legacy-database-import/src/probe_import.rs
- qualification/harnesses/legacy-database-import/src/report.rs
- qualification/harnesses/legacy-database-import/tests/fixtures.rs
- qualification/compatibility/legacy-database-import.json
- compatibility/fixtures/database/corrupt.sqlite
- compatibility/fixtures/database/empty.sqlite
- compatibility/fixtures/database/legacy-python.sqlite
- compatibility/fixtures/database/minimal-python.sqlite
- compatibility/fixtures/database/partial-python.sqlite
- compatibility/fixtures/database/unknown-schema.sqlite

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
| Status | accepted |
| Objective evidence reviewed | true |
| Normative constraints preserved | true |

## Immutable Consequence

&#40;none&#41;
