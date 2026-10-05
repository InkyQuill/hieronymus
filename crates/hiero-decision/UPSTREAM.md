# Upstream provenance

Primary donor: https://github.com/zchee/decision-model-sdk-rust
Revision: c5d4459f2e01bf72ccbf1b2cd55a801c039e7b07 (2026-10-04).

This is a narrowed, rewritten derivative of named question preparation
(`crates/sdk/src/question.rs::validate`) and answer dispatch
(`crates/sdk/src/de.rs`). It is not a source-compatible fork or full SDK port.
The donor's Apache-2.0 license and Python-derived MIT notice are retained
verbatim in LICENSE and LICENSE-THIRD-PARTY.

Hieronymus additions: probability/distribution validation, independent named
answer recovery, duplicate-key rejection, sanitized diagnostics and a fixed
64 KiB protocol cap. HTTP/TLS, authentication, deadlines and decision authority
remain in Hieronymus. There are no macros, async runtime, retries, body logging,
SIMD parser, model adapters or bundled HTTP client in this crate.

Useful external fixture ideas were reviewed from volker48 at
bf96dd87652964ce091186698a2b243fdd6ae86d (MIT). No volker source or fixture
files were copied into this crate; local tests exercise Hieronymus contracts.

The two installed project skills under .agents/skills were copied from that
fixed volker revision by skill-installer. Its skills-lock.json attributes
codebase-design and diagnosing-bugs to mattpocock/skills.

