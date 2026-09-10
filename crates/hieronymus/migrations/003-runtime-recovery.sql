-- Schema step 2 -> 3: durable runtime-recovery state.
--
-- Applied by `schema_upgrade::apply_steps` inside the caller's exclusive
-- transaction; it never commits and never runs outside one. The version
-- markers are the LAST statements in the file, so a failure anywhere above
-- rolls back before the new version is ever published.
--
-- Plain `create table` (no `if not exists`): the step runs exactly once per
-- version transition, guarded by the version marker, so an object already
-- squatting on one of these names is a real defect that must fail the
-- transaction rather than be silently accepted under a published v3 marker.
--
-- NO SEED ROWS. Every table here is a singleton whose ABSENCE is a meaningful
-- value ("no corpus change recorded", "no indexing owed", "no dream failure
-- streak"), and the Rust->Rust upgrade verification
-- (`migrate::verify_rust_upgraded_target`) requires every table a step
-- introduces to be EMPTY — an ordered schema step adds tables, it never adds
-- authoritative rows. Readers therefore coalesce the missing row to its
-- identity value and writers upsert (`rag::record_corpus_change`).
--
-- `corpus_revision` + `semantic_work_intent` close review finding A4 (task
-- C4): the pre-C4 import path committed its authoritative transaction, THEN
-- queued the semantic rebuild out-of-band, so a crash — or an enqueue
-- failure — in that window left new chunks unindexed while an OLDER
-- generation stayed `active`, and the controller reported `Ready` over text
-- that was not semantically retrievable at all.
--
--   * `corpus_revision.revision` is a monotonic counter bumped inside the SAME
--     transaction as every RAG change that alters indexable text. Every
--     candidate generation records the revision it was begun at
--     (`semantic_generations.corpus_revision`), which turns "does this
--     generation cover the corpus?" into an exact comparison instead of the
--     count-only guess that treated an equal-count document replacement as
--     already-covered.
--   * `semantic_work_intent` is the durable "an import happened, indexing is
--     owed" record, written in that same transaction. Losing the in-memory
--     worker wakeup after the commit can therefore no longer lose the work:
--     startup and periodic reconciliation read the intent from SQLite and
--     queue the rebuild even though *some* active generation exists. Only the
--     latest revision matters, so the row is coalesced by an upsert and
--     deleted once a queued or active generation covers it.
--
-- `semantic_generations.corpus_revision` is NOT altered here. That table is
-- created lazily at runtime (`semantic_store::ensure_semantic_schema`), so a
-- v2 database that never armed semantics does not have it and a blind `alter
-- table` would fail the whole upgrade. The column is added by the step's
-- typed converter instead (`schema_upgrade::add_corpus_revision_column`),
-- which alters the table only when it exists; a table created later carries
-- the column from its `create table if not exists` definition. Both paths
-- converge on `default -1`, the sentinel for "begun before revisions were
-- recorded" — such a generation must be REBUILT and is never relabelled with
-- a revision it cannot be shown to cover.
--
-- `dream_link_batches.next_left_offset` / `next_right_offset` and
-- `dream_retry_state` are the durable link-phase pair cursor and the dreaming
-- retry/backoff streak. The columns and the table land here (one schema
-- version, one upgrade) and are wired by task C8; `dream_retry_state` follows
-- the same no-seed rule, so C8 upserts its singleton rather than assuming a
-- row exists.

create table corpus_revision (
  singleton integer primary key check (singleton = 1),
  revision integer not null
);

create table semantic_work_intent (
  singleton integer primary key check (singleton = 1),
  revision integer not null,
  requested_at text not null
);

alter table dream_link_batches add column next_left_offset integer not null default 0;
alter table dream_link_batches add column next_right_offset integer not null default 1;

create table dream_retry_state (
  singleton integer primary key check (singleton = 1),
  failures integer not null,
  next_attempt_at text,
  config_fingerprint text not null
);

-- Version markers last: every object above exists before v3 is claimed.
update hieronymus_meta set schema_version = 3;
pragma user_version = 3;
