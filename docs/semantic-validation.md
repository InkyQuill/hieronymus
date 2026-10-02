# Semantic validation

[PR #119](https://github.com/InkyQuill/hieronymus/pull/119) replaces LanceDB with
SQLite and exact cosine ranking in Rust; main retains LanceDB until it lands. The model,
tokenizer and ONNX stack are unchanged. See [ADR 0013](adr/0013-semantic-index-and-platform-support.md)
and the [SQLite decision and checks](https://github.com/InkyQuill/hieronymus/blob/87a74f9029389ad670a950468aa66488554f7c47/docs/research/2026-10-02-sqlite-vector-decision.md).

The SQLite branch focused tests cover exact ranking, series isolation, full embedding identity,
transaction rollback, generation publication, corrupt/missing-index recovery and
preservation of old Lance artifacts. These tests establish storage behavior, not
real-model relevance or native agent-host acceptance.

For real inference and installed-artifact checks, use the disposable inputs and
explicit ignored-test commands in [runtime qualification](rust-cutover-rehearsal.md).
Record the tested commit, model identity, corpus, commands and failures with the
run. Do not treat an ignored test or an old qualification record as passing on a
new backend. Synthetic recall@k against an exact oracle measures neighbor-search
agreement; literary relevance requires representative text/query judgments.

Older Lance/model validation reports are available in
[Git history](https://github.com/InkyQuill/hieronymus/blob/6d1bc393c86a243b99742779b2c37f4591b2aa27/docs/semantic-validation.md).
Their conclusions apply only to their recorded source, assets and platforms.
