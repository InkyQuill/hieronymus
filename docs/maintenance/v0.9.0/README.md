# v0.9.0 maintenance and release evidence

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
