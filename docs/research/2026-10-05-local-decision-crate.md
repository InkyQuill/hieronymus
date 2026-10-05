# Local decision protocol migration — 2026-10-05

Approved after [the four-SDK audit](2026-10-05-decision-sdk-audit.md).

## Result

`crates/hiero-decision` replaces the unavailable registry package
`typesafe-sdk-rust = "=0.2.0"`. It is an unpublished, narrowed derivative of
zchee's audited `c5d4459` revision, not a full vendored SDK. Source provenance
and Apache-2.0/third-party MIT notices are retained under the crate and copied
into every native package by `scripts/desktop-package.ts`.

The protocol prepares current Noul/Choice/Score named requests, retains raw
question extension fields, validates probabilities/distributions and matches
answers to requested names. Valid answers survive invalid siblings; domain
thresholds, scope, vetoes, authority and fallback remain with existing consumers.
Duplicate response JSON keys and oversized bodies fail closed.
Metadata omission remains compatible with historical Hieronymus test fixtures.

The Jev adapter directly uses our synchronous provider transport. There is no
SDK HTTP/TLS client, scoped-thread async runtime, Tower service, implicit retry,
environment-selected endpoint or request/response logging. Credentials and
diagnostics remain separate; blank keys and header injection fail before sending.

The bounded transport entry point limits decoded bodies to 64 KiB and reserves
a finite `2 * body limit + 16 KiB` wire budget for headers/chunk framing.
Excessive framing still fails closed. DeadlineTransport forwards the same cap
and remaining shared deadline. Existing unbounded-provider entry points retain
their previous configured wire limits.

## Dependency reduction

The local crate has only two direct dependencies: `serde`, `serde_json`.
No new registry package was added.
Eleven registry package/version entries were removed:
`typesafe-sdk-rust`, `bytes`, `compact_str 0.10.0`, `http`,
`http-body`, `http-body-util`, `httpdate`, `secrecy`,
`serde_path_to_error`, `tokio`, `tower-service`.
Another existing compact_str version remains for other code.
Binary-size and release-build speed savings were not benchmarked.
The Rust 1.98 workspace baseline is unchanged.

## Project skills

Installed `codebase-design` and `diagnosing-bugs` with skill-installer into
`.agents/skills`, using audited volker revision
`bf96dd87652964ce091186698a2b243fdd6ae86d`.
The donor records their origin as `mattpocock/skills`.
This directory remains ignored according to the repository's existing policy.
They are installed locally and become discoverable in the next turn.

## Verification

- Nine new protocol tests: all three primitives, structured fields, partial
  malformed batches, invalid probability distributions/winners, score legend
  and weighted value, duplicate keys, invalid/trailing JSON, byte boundary,
  missing/unrequested answers and sanitized diagnostics.
- Six new application/real loopback tests: fixed endpoint/timeout/one request,
  no retry/redirect, credential/header validation before sending, response cap
  with injected transport, real TCP body boundary and whole-request timeout.
- Existing memory comparison suite: 16 passed, one live synthetic test ignored.
- Rust format and strict all-targets/all-features Clippy passed.
- Release/metadata scripts: 167 tests passed.
- Frontend: typecheck, 144 tests and production build passed.
- Native development build: CLI with embedded production console and desktop
  helper passed; `hiero version` was exercised without starting the server.
- Full Rust test checklist: 1740 passed, 20 ignored, zero failures. Ignored
  live-model and installed-artifact tests were not run.
- Strict all-features rustdoc passed.

Static independent review found no remaining blocker. Its initial body-vs-wire
cap observation was fixed and covered with actual TCP boundary tests.
No live model call, installed-server upgrade, release publication or
cross-platform execution was performed.
