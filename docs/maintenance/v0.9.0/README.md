# v0.9.0 maintenance and release evidence

## Published and verified (2026-09-13)

[v0.9.0](https://github.com/InkyQuill/hieronymus/releases/tag/v0.9.0) is published
from source/tag `d6d93f766e78cea09fb0573170e5c1019409f422`. PRs #26, #27, #28
and integration PR #29 are merged. Main was pulled locally and the Linux release
build completed successfully. This final audit update changes documentation only;
the release tag continues to identify the tested source.

- [Candidate run 34721365040](https://github.com/InkyQuill/hieronymus/actions/runs/34721365040)
  passed all four targets. Each extracted native package passed real model inference
  and authenticated MCP; Windows took 8.84 seconds, Apple Silicon 16.21 seconds,
  and Intel macOS 26.72 seconds for that explicit test.
- [Evidence run 34726943938](https://github.com/InkyQuill/hieronymus/actions/runs/34726943938)
  passed using immutable data commit `7bdea928a511b7acddde3d236356740e5311e158`.
  Its retained artifact exactly matched the locally validated evidence package.
- [Publisher run 34727072266](https://github.com/InkyQuill/hieronymus/actions/runs/34727072266)
  passed, promoting the retained bytes without rebuilding. All 28 published asset
  SHA-256 digests matched the expected candidates and qualification summary; see
  [final-release-verification.json](final-release-verification.json).

The current standalone installer installed the final Linux archive into a fresh
disposable root. Its installed-binary real inference/MCP test passed in 3.43 seconds.
Fresh KDE Wayland checks passed startup, duplicate-start protection, restart and
private-storage failure handling. A server started without graphical environment
variables owned an Active tray item through its helper; stop and restart cleaned up
the owned processes and tray items. Fresh browser pages loaded the author dashboard,
memory view and MCP-plus-skills connection flow without browser authentication.
No real agent host was configured or claimed connected by those browser checks.

Qualification remains partial on KDE Wayland and unqualified for the other six
desktop/session records. CI package tests do not establish physical Windows 11,
macOS 26.5 M1, Intel macOS, GNOME or KDE X11 desktop acceptance. Intel macOS is built
and explicitly unqualified, as requested. The published `native-qualification.json`
contains the exact passed checks and unavailable scenarios. Temporary local test
services and their tray helpers were stopped after testing.

Intermittent GitHub API polling connection failures were retried against the same
run; they did not fail or restart the candidate jobs. A local staging-directory
inspection used an unmatched shell glob; listing the directory corrected that
inspection without changing artifacts or qualification results.

## Final artifact transport correction (2026-09-12, 21:24 UTC)

PRs #26–#29 are merged. Source `b4f2cba5204480f56302e0cf537f1b35c32e9e88`
was fast-forwarded to main and pulled locally. Its complete native candidate run
[34713682687](https://github.com/InkyQuill/hieronymus/actions/runs/34713682687)
passed on all four targets, including packaged real inference and authenticated
MCP. Local main compiled successfully; installed Linux checks covered inference,
startup, duplicate ownership, tray registration, restart, insecure credential
refusal and unauthenticated browser overview/memory reads. The seven-session data
commit `4c07c95c0ab56cc6e691672c7fd1b433b963ffbb` preserves those exact-byte
observations and explicit gaps; it does not qualify subsequent candidates.

The evidence run
[34719821220](https://github.com/InkyQuill/hieronymus/actions/runs/34719821220)
failed before matrix validation, in the candidate download loop. Its wrapper
reported `GitHub artifact operation failed` at `scripts/desktop-ci.ts:172`.
A direct reproduction exposed the cause: extracting the Apple artifact into the
already downloaded Linux directory failed on `desktop-metadata.awk: file exists`.
Every platform carries shared model/installer files, and GitHub CLI refuses even
identical existing files during extraction.

The downloader now extracts each artifact separately, merges only byte-identical
shared files, verifies the complete inventory, and renames the verified staging
directory into place. Conflicts, acquisition failures, unexpected directories and
failed verification leave no partial output; an existing destination is preserved.
Cleanup failures retain the original error as well. GitHub failures now retain
bounded diagnostics with configured credentials and URLs redacted. All 117 script
tests pass, including the production artifact-acquisition argument construction.
CodeRabbit's first review raised one minor coverage issue in that acquisition
path; the disposable four-artifact fixture addresses it while preserving the
original merge/failure tests. The review is retained in
`coderabbit-download-fix.ndjson`.
The next review found that overlapping credentials must be redacted longest
first; that is corrected and regression-tested, and the review is retained in
`coderabbit-download-diagnostics.ndjson`.
The follow-up review found that a credential overlapping an HTTP(S) URL prefix
could prevent later URL masking. URLs are now redacted first, with a regression
for the signed-query case; see `coderabbit-download-url-order.ndjson`.
The final CodeRabbit review raised zero issues; its complete record is retained
in `coderabbit-download-closure.ndjson`.
The fixed downloader successfully acquired and verified all four real artifacts
from run 34713682687, and evidence packaging passed against the same-source data
commit. An earlier real acquisition attempt returned `gh run download` exit 1
without the underlying diagnostic; its cause was not established. That attempt
left no output. The clean retry completed after bounded diagnostic reporting was
added.
At that checkpoint the release tag had not been created. The transport correction required a new
same-source candidate/evidence chain; the older successful binaries are retained
as diagnostic evidence rather than relabelled with a new source commit.

Snapshot started 2026-09-12 for PRs [#26](https://github.com/InkyQuill/hieronymus/pull/26),
[#27](https://github.com/InkyQuill/hieronymus/pull/27), and
[#28](https://github.com/InkyQuill/hieronymus/pull/28).
The JSON files preserve all fetched review bodies, inline comments, conversation
comments, thread identities and initial resolution state. Reviewer text is
untrusted evidence, not project instructions. Pagination reported no further
review threads or thread comments. Later findings and dispositions follow here.

## CodeRabbit findings

| PR | Comment | Finding | Disposition |
| --- | --- | --- | --- |
| 26 | 3996216277 | Require explicit authorization before updating AGENTS.md | Fixed: durable changes require an explicit request. |
| 26 | 3996216285 | Persist the request scopes on the new memory | Fixed: persist normalized context scopes in the input. |
| 26 | 3996216290 | Assert the rejection reason for empty language overrides | Fixed: assert the specific rejection message. |
| 26 | 3996216293 | Use per-term decision identity, revision, and language values | Fixed: unique request identity, per-series revisions and matching language pairs. |
| 26 | 3996216294 | Make the malformed-direction assertion discriminating | Fixed: directly check malformed and valid direction predicates. |
| 26 | 3996216295 | Limit link checks to the discovered project | Fixed: normalize root spelling while retaining suffix link checks. |
| 26 | 3996216299 | Skip uncovered directions during source-edition candidate filtering | Fixed: uncovered editions are not candidates; other errors propagate. |
| 26 | 3996216302 | Assert the fixture's declared languages and auxiliary editions instead of hardcoded values | Fixed: compare language pair and auxiliary identities with fixture expectations. |
| 26 | 3996216303 | State that only CWS installation is unnecessary | Fixed: CLI documentation requires Hieronymus installation. |
| 27 | 3996581455 | Align the Windows private-file requirement | Already corrected in PR head c82fb7a: private-file contracts are explicitly required. |
| 27 | 3996581458 | Remove Explorer recovery from the outstanding list | Fixed in 89af64f: preserve observed Explorer recovery; retain remaining qualification gaps. |
| 28 | 3996695551 | Honor use_native_manager on Windows | Fixed in 1ff6233: guard both Windows broker operations. |
| 28 | 3996695556 | Guard the entire test file on Unix targets | Fixed in 1ff6233; reconcile file-level guard with #27 native fixtures during merge. |

### Integration PR #29 review

Full review bodies and comments are retained in `pr-29-reviews.json`,
`pr-29-inline-reviews.json` and `pr-29-conversation.json`.

| Comment | Finding | Disposition |
| --- | --- | --- |
| 3996969246 | Fixture provenance claimed byte identity despite local additions | Corrected provenance wording. |
| 3996969258 | Registry language defaults were not normalized | Normalize both paths. Preserve unspecified default languages for ordinary writing-memory sessions; reject explicit empty overrides. Regression added. |
| 3996969264 | Display-variable startup gate skipped user-service trays | Use trusted graphical-session discovery, recover only display settings from the private user manager environment, and retry after later login. Live no-display-variable service test passed. |
| 3996969266 | Non-UTF-8 project roots could panic during JSON projection | Reject unsafe reported paths before serialization; test implicit non-UTF-8 cwd and graceful rejection of invalid UTF-8 arguments. |
| 3996969271 | Windows uninstall fixtures were compiled out | Restored fixtures and tests, sorted retained-name expectations, and added explicit native CI execution. |
| 3996969273 | Common trailer made skill-body assertions vacuous | Strip trailer before checking each body; corrected remember skill wording exposed by the stronger test. |
| 3996969277 | Blocking loopback fixture reads could stall teardown | Bounded read/write waits, whole-header deadline and byte limit; failed reads do not serve success responses. |
| 3996969279 | Gate the daemon-backed semantic evidence fixture | Retained ordinary coverage: this test exercises authority/evidence helper bookkeeping, not inference. Module documentation now explicitly distinguishes it from opt-in real-model qualification. Static companion review agrees. |
| 3996969282 | Windows invalid-credential fixture depended on inherited ACLs | Grant an explicit Everyone read ACE and check command success; run worker contracts in native CI. |
| 3996969286 | Missing padded-direction predicate test | Added padded and empty-identity rejection assertions. |
| 3996969287 | Unicode/control line separators survived line splitting | Strip all recognized separators; test blank lines and closing delimiters for each. |
| 3996969291 | ADR decisions conflicted with optional browser authentication | Updated normative decisions and consequences for both browser modes and existing waived CSRF/rotation rules. |
| 3996969292 | Null qualification was treated as omitted | Reject present null; regression added. |
| 3996969295 | Intel could be incorrectly labelled qualified | Enforce unqualified Intel desktop evidence; regression covers qualified and partial rejection. |
| 3996969298 | Reviewed runtime inputs were verified after output publication | Verify archive and receipt before packageDesktop writes outputs. |
| 3997029212 | Raw audit identity could split tray ownership from the validated graphical session | Remove the audit-only shortcut; supervisor and helper use the same logind validation. Check shutdown again immediately before spawning after environment recovery. |
| 3997029216 | Rejected public runtime downloads remained on disk before CI fallback | Remove the attempt's private temporary directory before creating the independent fallback directory. |
| 3997214344 | Require a reviewed status inside the retained producer receipt | Not applied: the receipt is immutable build evidence. Explicit approval is the compiled source-reviewed authority plus its exact receipt hash. Changing producer status would alter the retained evidence. Documented this boundary and added a regression showing that producer status cannot grant authority. |
| Review 5187678105, outside-diff comment | Invalid retained artifacts could leave their temporary directory behind | Fixed: clean every failed acquisition/validation path; retain the original error, including when cleanup also fails. Four isolated fault-injection checks pass. |

The strengthened skill test initially failed on `hieronymus-remember` because
its body said “free-text agreement”; it now names the project agreement explicitly.
The initial non-UTF-8 CLI fixture also exposed `std::env::args()` panicking before
project inspection; arguments now fail with a normal diagnostic, and the working
directory case returns the structured invalid envelope. These failures were
fixed, not waived. Focused CWS domain tests passed (22), as did the five focused
CLI/provider/worker integration suites and 109 release-script tests.

The companion review additionally found a possible pipe-capacity deadlock in
session environment capture. Nonblocking bounded draining now runs during child
polling, with tests for larger-than-pipe output and excess output. Final static
review found no remaining correctness issue in that helper.

Live CachyOS/KDE Wayland development test: transient service
`hieronymus-qualification-nodisplay-20260912.service` launched pid 211171 with
DISPLAY, WAYLAND_DISPLAY and XAUTHORITY absent. Helper pid 211245 recovered those
three settings and registered an Active StatusNotifierItem. Authenticated graceful
stop removed both processes and returned the watcher from 8 items to its baseline
7; the transient unit became inactive. This is development-source evidence,
not qualification of the future immutable release candidate.

PR #26 passed every check and merged at 2026-09-12T17:32:53Z as
`7eed43b72a3af0d45366dbe6429b7c604848b845`.

PR #27 passed every check, including native Windows and Intel macOS, and retained
CodeRabbit approval. It merged at 2026-09-12T17:51:09Z as
`f97764178eda13fe5bc9c7cf780a7491f502b26b`.

The next full Rust run hit an intermittent message assertion in
`update::tests::undiscovered_owner_refuses_before_retirement_or_manager_actions`
(162 library tests passed, 1 failed). The original assertion did not print the
returned error. After adding diagnostic output, ten complete library-suite
reruns passed, so the precise race remains unconfirmed. The fixture now holds
the actual RootOwnership lock without starting unrelated daemon workers and
deleting their discovery state; all existing no-manager/no-retirement/no-switch
assertions remain. Its focused test passed. The README installation instructions
also now describe the v0.9 split archives and matching desktop installers.

The subsequent full run exposed an overly strict review fix in language-default
normalization: `recall_exposes_contract_even_when_results_are_empty` failed with
`source_language must not be empty` (13 other application-memory tests passed).
Ordinary writing-memory projects may omit translation languages. Defaults are
now normalized without requiring a language pair; explicit empty overrides still
fail. The focused language test and all 14 application-memory tests passed.

Current follow-up validation passed the full normal Rust suite (1,558 passed,
17 intentionally ignored) and warning-denied rustdoc, as well as
all-target/all-feature Clippy and 109 script tests. A trailing
helper command used the incorrect package name `hieronymus-desktop` and failed
before selecting tests; the separate correct `hiero-desktop` command passed
Clippy, all 20 helper tests and warning-denied rustdoc. This command error did
not invalidate or replace the successful main-workspace test results.

The revised shared graphical-session resolver also passed a fresh live KDE
Wayland check: transient unit `hieronymus-qualification-session-final-20260912`
started daemon 251066 without display variables, helper 251141 registered an
Active tray item, and authenticated stop removed both processes and returned
the watcher from 8 items to its baseline 7. The unit ended inactive. An auxiliary
request to the nonexistent `/api/status` route yielded no evidence; this rerun
claims tray lifecycle only. Earlier valid no-cookie web checks remain separate.

CodeRabbit completed its review through `2242c7d` at 2026-09-12T17:55:41Z with
no new actionable comments and no identified merge-blocking risk; both follow-up
findings are marked addressed. GitHub's aggregate review decision still shows
the older changes-requested review. Its generic docstring-coverage warning
(61.04% against 80%) is retained in the full conversation snapshot; this is not
a failing Rust documentation check, and does not justify mechanically adding
comments to private helpers. Warning-denied rustdoc passes.

The companion Rust reviewer rechecked the final shutdown fix in `5825531`:
the stop flag is checked after environment recovery immediately before spawn,
and both session lookup routes apply the same graphical validation. No remaining
actionable findings in that bounded read-only review; no tests were rerun.

Intel runtime build 34703835119 / job 103580591056 completed its native build
and all 10 upstream CTest groups, then intentionally failed staging at the
unpromoted-runtime gate at 2026-09-12T18:45:14Z. Artifact 10303049116 retains the
archive and measured receipt. Reviewed source is ONNX 1.28.0 commit
`da9b5e364c465de65c49d91e696cd6485270757f`; all archive/member/log hashes match.
Independent LLVM inspection confirms x86_64, Mach-O minimum macOS 14.0, SDK 15.5,
and only Apple system dependencies. Native toolchain: macOS 15.7.9, Xcode 16.4,
Apple clang 17.0.0. Upstream skipped/disabled tests remain explicit in build.log;
this is not Hieronymus final-model or interactive-desktop qualification.

The exact archive digest is
`ff8befc83b955526ccf4062686d50d86796c9123d712ec1ec6c671fc97f330eb`;
the receipt digest is
`331a510ab904dfde7e6fdc8fd692046e4f548d6c57428ef205bef471a1588994`.
The small receipt, build plan and Mach-O reports are retained alongside this
ledger. These reviewed bytes now replace `source-build-required` authority.
Acquisition from the retained CI artifact passed and reverified the compiled pins.

Pin promotion exposed a synthetic candidate-inventory fixture missing the newly
required Intel archive and receipt (108 script tests passed, 1 failed with
ENOENT). The fixture now supplies independently hashed synthetic runtime inputs
and tests archive and receipt substitution, with production callers retaining
compiled runtime authority. All 109 script tests pass after the fixture fix.

Integration run 34709507741 at `2242c7d` passed frontend, full backend, Linux,
Windows, Apple Silicon and Intel jobs. Windows logs confirm all five restored
uninstall unit tests and all ten worker-lifetime tests executed successfully.
The subsequent pin-only release change also passed real retained-artifact
acquisition and strict staging with the canonical pinned model; native
Hieronymus inference still belongs to the final four-target candidate run.

Candidate attempt 34712698827 used `016d0bb` and was superseded before promotion
to retain the latest review disposition and authority regression in the source.
Its artifacts are not final candidates. The regression suite passes 110 tests.
The new-pin Rust validation separately passed 1,558 tests (17 intentionally
ignored), Clippy and warning-denied rustdoc; the follow-up changes only document
and test the existing script authority boundary.

Review 5187678105 identified the retained-download cleanup gap. Candidate attempt
34712859698 at `075d0b6` was cancelled because this valid fix supersedes its source.
The full acquisition is now enclosed in failure cleanup, including nonzero
download exit, missing receipt, corrupt receipt and corrupt archive. An isolated
fake downloader exercised all four cases: original failures returned and no
acquisition directories remained. All 110 script tests pass. Cleanup failure
preserves both errors in an AggregateError instead of obscuring the original.
Real retained-artifact acquisition also passed after this change and preserved
the returned archive and receipt for packaging.

CodeRabbit automatically paused GitHub reviews after the active-development
commit threshold; its green status alone does not review `80555b5`. The installed
authenticated CLI (0.7.5) then reviewed the committed scripts diff from `075d0b6`
through `80555b5`, using AGENTS.md, and completed with 0 issues in
`scripts/reviewed-runtime.ts`. The complete structured result is retained as
`coderabbit-cleanup-final.ndjson`. This final audit update changes only evidence;
the reviewed acquisition implementation is unchanged.

## Observed run errors

Candidate run [34700290410](https://github.com/InkyQuill/hieronymus/actions/runs/34700290410)
built macOS branch source `528796a24e353ee0025a5f4293e6746c95e337f2`:

- Linux job 103570847455: Clippy rejected an unnecessary mutable binding in
  `hiero`. The subsequent macOS branch commit `b860c4c` gates mutations by target;
  verify after integration.
- Windows job 103570847471: `validated_handle_keeps_its_secret_when_the_path_is_replaced`
  failed at `private_file.rs:130` with Win32 error 5 (Access is denied), 3 passed,
  1 failed. The Windows PR changes native replacement and this contract;
  verify the combined candidate.
- Intel macOS job 103570847529: native ONNX build failed because generated
  `onnx-ml.pb.h` was incompatible with the Protocol Buffer headers. This caused
  undefined `PROTOBUF_NAMESPACE_OPEN` and `Arena` errors and make exit 2.
  Investigate the pinned runtime build's compiler/header selection.
- PR run 34702651411 was cancelled when superseded by 34702862911; cancellation
  is not a passing test result.

Native source qualification failures and limits are also preserved in the PR
bodies and their linked Windows/macOS receipts. Source-tree or synthetic passes
must not be represented as qualification of final retained release bytes.

## Validation and release

In progress. No release qualification or publication is claimed by this record.

### Practical release scope approved 2026-09-12

The available test hosts are CachyOS/KDE, Windows 11 and macOS 26.5 on M1.
Pavel requested testing what these hosts can cover, and explicitly requested
including an Intel macOS build labelled unqualified. Unobserved environments
and checks must remain visible gaps; they must not be converted to passing
observations. Adapt the release gate to this scope while preserving exact
source, artifact integrity, mandatory semantic inference and failed-check rejection.

### Focused CWS validation

Linux domain suite: 21 passed. CWS direction, plugin and project-context suites
passed. The companion static Rust review found two edge cases in the initial
path/filtering fix; both were fixed and now have regression coverage.

The obsolete candidate run 34700290410 was cancelled after its three failures
were archived, to allow corrected run 34703835119 to start. Intel build.log
shows pinned protoc 21.12 with discovered Homebrew Protobuf 36.0.0; commit
1ff6233 disables dependency package discovery so ONNX uses its pinned sources.

### Follow-up review and integration validation

CodeRabbit follow-up 3996779881 (PR #26) found that the archived record for
34700290410 still said in progress after cancellation. Its terminal status is
now recorded as completed/cancelled. The follow-up review snapshot is preserved
in `review-threads-followup.json`; the original 13 review threads were resolved
on GitHub by the time of that snapshot.

Diagnostic candidate 34703835119, Windows job 103580591063, reproduced
`validated_handle_keeps_its_secret_when_the_path_is_replaced`: AccessDenied (5)
at private_file.rs:130. That candidate was built from the macOS branch and did
not include PR #27's Windows fix. The integrated branch does include that fix;
final integrated Windows verification remains required.

Integration validation found and corrected these mismatches:

- The first auth build missed the new LocalConsole case in authority_signal.rs.
- Desktop-state tests still expected a stopped server to leave a Start tray.
- The navigation test still expected three links after adding agent connection.
- The unauthenticated WebSocket handshake test expected 426; the existing route
  contract returns 400 with websocket_upgrade_required.
- The distinct-browser-principal authority test needed explicit auth enabled;
  default local desktop mode intentionally uses local-console attribution.
- The first graphical-session implementation used a macOS-only libc dependency
  on Linux; it now uses the already-pinned rustix crate's process feature.

The full normal Rust suite passed after the auth/state fixture fixes. Subsequent
tray session tests passed (4), and Clippy passed before the final custom desktop
entry change. Custom registration/native/CLI tests then passed. Frontend typecheck,
86 tests, and production build passed. Release script tests passed (103).
These are development-source checks, not final-byte native qualification.

The companion Rust review identified and drove fixes for stale config reads by
the browser launcher, hidden helper exit errors, custom service directory
propagation, and the logind identity of user-service children. Live KDE tray
acceptance is still pending. See `product-direction.md` for the owner's revised
application contract and practical release scope.

Live development-build checks on CachyOS/KDE Wayland then passed: direct server
startup and a disposable systemd user service both created an Active
StatusNotifierItem, served the no-cookie web API, and removed their tray items
and helper processes after graceful server stop. The user-service test used
logind's validated graphical-session fallback. No login registration was added;
the transient test unit finished inactive. The rendered connection page was
opened in the in-app browser, prepared successfully, and displayed pi setup
instructions covering MCP and skills. This is not final-release-byte evidence.

Real pinned ONNX/model qualification passed (1 ignored-by-default integrated
suite explicitly executed, 110.54 seconds), including multilingual corpus and
failure/recovery scenarios. The stable Linux launcher review fix passed its
installed-layout regression (6 service CLI tests). Final full-suite revalidation
then found the tray argument fixture still expecting two argv entries rather
than the six required for data root, service directory and stable binary; the
fixture now checks all literal arguments, including spaces and shell-like text.

The final local verification chain passed main-workspace fmt, all-target/all-feature
Clippy, normal Rust tests and rustdoc, then helper Clippy. Helper tests exposed one
remaining stale expectation that Start stays enabled after Event::Stopped. That
assertion now expects disabled Start and requested helper exit; the complete
helper suite (20 tests) and helper rustdoc passed on rerun. No functional checks
were removed. Source revalidation also passed 103 script tests and 86 frontend
tests with typecheck/build. Optimized local Linux packaging is running separately.

PR #28 merged into main as 6c92f728e9626d88527736c805fc6a745b3721ba after all
checks and CodeRabbit approval. PR #27 then merged that main state locally,
retaining the integration branch's previously verified native conflict
resolutions, and pushed 0acaf5c. PR #29 contains the additional server/tray,
onboarding, optional-browser-auth and practical qualification changes.

Optimized local Linux packaging completed successfully. The extracted, verified
package passed the explicit installed-payload test with real semantic inference,
authenticated MCP and graceful shutdown (1 test, 3.36 seconds). These local
development-candidate bytes are not the later immutable publication candidate.

Reviewed Intel runtime support now binds the source receipt, retained CI run,
archive digest and every member digest. Published releases retain that archive
and receipt so acquisition can outlive CI artifact retention. The first release
can acquire the same pinned bytes from the retained native build, with a bounded
download timeout. Runtime pins remain `source-build-required` until the native
build output is available and reviewed. The script suite passes 107 tests,
including stalled-download and failed-download rejection. Installer shell syntax
validation also passes with the Intel target enabled.

CodeRabbit review of PR #29 initially hit the included-review rate limit; its
original-PR approvals do not constitute review of the integration changes.

After adding reviewed-runtime support, full local revalidation passed again:
1,553 normal Rust tests (17 intentionally ignored), all-target/all-feature Clippy,
warning-denied rustdoc and 107 script tests. The final-payload inference test
above remains separate from the normally ignored suite. CodeRabbit subsequently
started the PR #29 review. The diagnostic Intel job passed private-file tests and
began its native runtime source build at 2026-09-12T17:07:58Z.
