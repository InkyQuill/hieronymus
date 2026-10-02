# Documentation

Hieronymus runs on Rust. Current work is optimization and behavior fixes; see the
[roadmap](roadmap.md), including deferred 1.0 work.

## Using and operating Hieronymus

- [Usage](usage.md), [installation and distribution](distribution.md)
- [Agent workflows](agent-workflows.md), [authority ingress](authority-ingress.md)
- [Memory and Dreaming](memory-dreaming.md)
- [Desktop platforms](desktop-platforms.md), [tray](desktop-tray.md)
- [Automatic releases](automatic-releases.md), [CI checks](ci-checks.md)

## Engineering contracts and verification

- [Project rules](../AGENTS.md) and [conventions](project-conventions.md)
- [Product direction — ADR 0016](adr/0016-autonomous-story-memory-product-vision.md)
- [Semantic storage — ADR 0013](adr/0013-semantic-index-and-platform-support.md)
  and [SQLite decision](research/2026-10-02-sqlite-vector-decision.md)
- [Semantic validation](semantic-validation.md),
  [disposable runtime qualification](rust-cutover-rehearsal.md)
- [Agent-host acceptance](agent-host-acceptance.md) and
  [deferred qualification and quality work](deferred/2026-09-10-native-host-qualification.md)

The remaining ADRs preserve architecture decisions and their amendments. Read
amendments before older text. Retained Rust/authority/desktop specifications in
superpowers/specs/ describe detailed contracts, not unfinished port tasks;
current code, tests and amended ADRs take precedence over obsolete phase wording.
They are retained where their contract has not yet been replaced by a concise
maintained guide. Qualification procedures do not establish a passing result.

## History

Completed execution plans, superseded UI ADRs, Python baseline documents and
one-off reports/receipts have been removed. They remain available at
[the pre-cleanup revision](https://github.com/InkyQuill/hieronymus/tree/6d1bc393c86a243b99742779b2c37f4591b2aa27/docs).
Use git log --all -- docs/<path> and git show <revision>:docs/<path> to retrieve
an earlier document. Historical citations use immutable Git links; they are not
current release requirements. Frozen compatibility fixtures and qualification
harness inputs remain untouched.

Keep current instructions, accepted decisions and unresolved work here. Put
one-off CI results in run artifacts or issue/PR records rather than accumulating
copies of logs and completed task reports in the documentation tree.
