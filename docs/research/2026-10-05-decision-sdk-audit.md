# Rust decision SDK audit — 2026-10-05

## Approved direction and implementation

The user approved this direction on 2026-10-05. The local
[`hiero-decision`](../../crates/hiero-decision/UPSTREAM.md) now replaces the
registry dependency; the two selected skills are installed project-locally.
The original audit below records the pre-change evidence. Implementation
verification is recorded in [the migration report](2026-10-05-local-decision-crate.md).

Build a narrow, unpublished workspace crate, provisionally `hiero-decision`.
Use **zchee as the primary implementation donor**, selectively adapting
volker48's protocol fixtures and error-path tests. Do not merge four SDKs or
vendor zchee's entire expanding workspace. Keep the existing dependency until
the local replacement passes our adapter and domain tests.

This is a recommendation, not an implemented migration. Production manifests,
the lockfile, runtime code, installed server and release configuration were not
changed. Research was performed against Hieronymus checkout
`3f643a2e75b44b723d0418f84ba7b8e1449b2a3b`; the failing remote release has a
different head.

## Confirmed release symptom

[Candidate run 37277637272](https://github.com/InkyQuill/hieronymus/actions/runs/37277637272),
head `a227309704d2afbb8c4b4c3f20ff3e6ccfaa78c9`, failed on Linux, Windows,
Intel macOS and ARM macOS with:

```text
error: no matching package named `typesafe-sdk-rust` found
```

Our manifest pins registry package `typesafe-sdk-rust = "=0.2.0"`;
`Cargo.lock` records its registry checksum. Current zchee sources declare
`decision-model-sdk = "0.1.0"` and library import `decision_model_sdk`.
This changes package identity and versioning, not just the GitHub URL.
Local cached builds do not establish availability on a clean runner.
The crates.io HTTP API returned 403 during this audit, so registry deletion,
yank status and author intent were not independently established.

## Fixed revisions and upstream results

All four clones were made in a disposable directory. Tests used synthetic
state and loopback servers; no live model calls or manuscript data.

| Project | Audited commit | Package / declared minimum Rust | Upstream tests passed / ignored |
| --- | --- | --- | --- |
| [netf](https://github.com/netf/typesafe-sdk-rs) | `1b9e488af1d6055766008c694f1fbb213e0d82a8` | typesafe-sdk 0.1.0 / 1.87 | 190 / 6 |
| [volker48](https://github.com/volker48/typesafe-sdk-rust) | `bf96dd87652964ce091186698a2b243fdd6ae86d` | typesafe-sdk 0.1.0 / 1.96 | 43 / 0 |
| [zchee](https://github.com/zchee/decision-model-sdk-rust) | `c5d4459f2e01bf72ccbf1b2cd55a801c039e7b07` | decision-model-sdk 0.1.0 / 1.98 | 1181 / 0 |
| [codeitlikemiley](https://github.com/codeitlikemiley/typesafe-sdk-rust) | `6eb6e104218d680000f6d2601dfd99d5b6735321` | typesafe-sdk 0.2.0 / 1.85 | 59 / 1 |

Counts sum lib, integration and documentation result lines, including volker's
TLS child processes. zchee counts include its default SDK, macros,
test-support and adapter members, **not live-test workspace members**.
Counts are evidence of execution, not comparable quality scores.
Ignored tests are not passed tests.

Each clean upstream checkout passed:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo test --all-features --locked`
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked`

Rust checks used `CARGO_BUILD_JOBS=2` and a disposable shared target directory.
For tests, `TYPESAFE_API_KEY`, `TYPESAFE_BASE_URL` and
`TYPESAFE_DEFAULT_MODEL` were removed from the inherited process environment.
The first netf run failed its no-key configuration test with the developer
environment inherited; serial execution did not fix it. Removing those
variables did. This is environment sensitivity, not a demonstrated race.
netf has no committed lockfile; its first resolution created a disposable
lockfile, then the recorded baseline and checks used `--locked`.
Declared minimum Rust versions were not separately qualified.

volker additionally passed **215/215 recorded Python-oracle scenarios**:
40 System One, 68 foundation, 67 models and 40 extensions.
The upstream harness validates fixture hashes/provenance and runs the compiled
Rust adapter against local HTTP scripts. This audited the recorded oracle,
not a fresh Python-service equivalence run. Its separate TypeScript E2E runner
was not installed or executed.

## Identical adversarial probes

The same loopback fixture tests were adapted to each public SDK API.
Requests set explicit fixture keys, model, base URL, 300 ms per-call timeout
and zero SDK retries. zchee additionally sets its available body cap to
65,536 bytes. Other SDKs expose no equivalent cap in the inspected clients.
The oversized fixture is valid JSON with 70,000 padding characters.
Redirect testing uses HTTP 307 to the same local origin and counts requests.

| Probe | netf | volker48 | zchee | codeitlikemiley |
| --- | --- | --- | --- | --- |
| Valid named Noul response | pass | pass | pass | pass |
| Invalid JSON rejected | pass | pass | pass | pass |
| One malformed answer rejects typed batch | pass | pass | pass | pass |
| 429 sends exactly one request with retries disabled | pass | pass | pass | pass |
| 307 does not trigger another request | pass | pass | pass | **fail: 2 requests** |
| Response above Hieronymus 64 KiB contract rejected | **fail** | **fail** | pass | **fail** |
| Noul probability 1.5 rejected | **fail** | **fail** | **fail** | **fail** |
| Client Debug hides fixture key, separately tested | not run | not run | not run | **fail** |

The deliberately strict probes therefore exit nonzero:
netf 5 pass / 2 fail; volker48 5 / 2; zchee 6 / 1;
codeitlikemiley 4 / 4 including its additional Debug probe.
These are application-contract gaps, not claims that upstream test suites fail.
Accepting numeric values outside probability bounds may match Python parsing,
but does not satisfy Hieronymus decision validation.

The malformed-batch result means typed decoding alone cannot retain valid
siblings. Our existing adapter recovers bounded raw JSON on response-validation
errors and independently validates each named answer. Preserve that behavior.
The common probes establish whole-batch rejection; they do not themselves
qualify every SDK's raw-error recovery interface.

## Why zchee is the donor

- It already fits Hieronymus's `ProviderTransport` through
  `ClientBuilder::build_with_service`, so no second HTTP/TLS stack is required.
- Prepared named questions, raw extension fields, bounded response collection,
  raw response metadata and explicit retry settings match our present adapter.
- Its SDK can disable Hyper, macros and tracing. The other three implementations
  directly own reqwest; accepting a custom reqwest client is not a transport
  interface compatible with our blocking, mockable provider transport.
- It has substantial real loopback, codec differential, property, compile-time
  and protocol tests. Its adapter also has recorded provider fixtures.
- Its current workspace also contains general OpenAI/Anthropic/Gemini adapters,
  derives, performance instrumentation and an optional SIMD codec.
  Those expansions are outside our current need and should not become our
  release dependency merely because they exist.

zchee is not certified stable by this audit: its probability decoder still
needs application validation, the published rename broke our clean release
resolution, and no new release/install or live-model qualification was run.
Stars, forks and the author's agent choice were not used as quality criteria.

## What to take from the others

**volker48:** strongest second source for a compact protocol implementation
and recorded compatibility fixtures. Useful cases cover omitted/null values,
error paths, score-key collisions, cancellation, TLS rejection, protected
headers and whole-attempt timeout behavior. Keep our transport seam; do not
import its reqwest client or add Python/Node compatibility gates to Hieronymus
release automation.

**netf:** useful synchronous API and public documentation examples, but it owns
reqwest and unbounded response collection. Its no-key tests also need a clean
environment. Consider documentation/fixture ideas, not its transport as our base.

**codeitlikemiley:** useful `wire` types and request/response round-trip tests.
The default client permits redirects and its derived `Client`/`Config`
Debug includes the API-key String. The latter was reproduced using
`AUDIT_FAKE_KEY_7129`, never a real secret. Neither behavior should be copied.

All three are MIT-licensed at the audited roots. zchee declares Apache-2.0
and includes a third-party MIT notice for its Python-derived material.
Any actual code/fixture copying must retain applicable licenses, notices,
source repository, commit and a brief derivation record.

## Proposed local crate scope

Expose a small protocol interface: validated named questions, request encoding,
per-answer parsing and fixed diagnostic categories. Support current Noul,
Choice and Score usage plus explicit raw fields where required.

Keep application policy in Hieronymus: evidence authority, termbase protection,
scope checks, comparison vetoes, configured thresholds and fallback decisions.
Preserve named answer matching and partial failure without re-requesting valid
siblings. No automatic retries, endpoint environment overrides, general
provider adapters, derive macros, tracing payloads or second TLS client.

Reuse the existing bounded provider transport. Enforce the intended Jev cap at
the read boundary as well as before parsing; our generic provider transport
currently has a broader 10 MiB default, so removing the SDK's 64 KiB check
without a replacement would weaken the contract.

Add deterministic rejection of nonfinite/out-of-range probabilities,
unsupported winners, invalid/missing distributions and other current domain
constraints. Define duplicate-name/key policy explicitly instead of inheriting
whatever the JSON map parser happens to retain.

Before replacing the dependency, qualify the local implementation against the
existing relevance and comparison/batching tests plus the probes here:
timeout, response cap, redirects, no retry, key redaction, named mapping,
malformed sibling recovery and probability validation. Run the project's normal
verification checklist after implementation. A live synthetic check is a
separate disclosed qualification; transport success is not model accuracy.

## volker's project skills

Inspected skill files and `skills-lock.json`; did not install or activate them.
Most are imported from **mattpocock/skills**; `typesafe-ai` comes from
**typesafe-ai/skills**, not an original SDK-specific technique.

- `codebase-design`: useful small-interface/deep-module and dependency-injection
  guidance for the local crate. Avoid importing its mandatory private vocabulary
  or speculative layers wholesale.
- `diagnosing-bugs`: useful repeatable failing reproduction before a fix,
  minimizing inputs and sharpening feedback. Its absolute workflow prescriptions
  need adapting to our existing practices.
- `domain-modeling`: useful explicit terminology and documenting resolved
  semantics. No need for another parallel glossary beside our product contracts.
- `typesafe-ai`: sound live-docs-first guidance, primitive selection, batching
  independent questions and distinguishing typed output from correctness.
  Use the authoritative origin if we later adopt/update it.
- `improve-codebase-architecture`: a broader interactive architecture workflow,
  with agent/UI/report assumptions. No demonstrated need to install the full
  skill collection for this SDK decision.

## Evidence and reproduction

[Logs and probe sources](sdk-audit-2026-10-05/) are retained in this repository.
Baseline checks were performed **before** injecting `tests/hiero_audit.rs`.
For each pinned checkout, copy its corresponding probe there; for zchee copy
to `crates/sdk/tests/hiero_audit.rs`. Then run:

```sh
env -u TYPESAFE_API_KEY -u TYPESAFE_BASE_URL -u TYPESAFE_DEFAULT_MODEL \
  CARGO_BUILD_JOBS=2 cargo test --all-features --locked \
  --test hiero_audit -- --test-threads=1
```

Nonzero exits are expected at the audited revisions, as tabulated above.
Remove injected probes before evaluating upstream formatting or baseline suites.
For volker oracle reproduction, build `--example compat_adapter`; its
`compat/e2e.py --suite <suite> --case <id>` checks each recorded case using
the executable under its `target/debug/examples` directory. The full 215-case
execution log is included. No compatibility fixtures were re-recorded.

Primary contract: [TypeSafe HTTP API](https://docs.typesafe.ai/api).
The API describes named questions/answers, Noul 0–1 probabilities and Choice
probability distributions. This remains the protocol authority; a community
SDK or another model's adapter is an implementation.
