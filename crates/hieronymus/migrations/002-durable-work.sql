-- Schema step 1 -> 2: durable work tables.
--
-- Applied by `schema_upgrade::apply_steps` inside the caller's exclusive
-- transaction; it never commits and never runs outside one. The version
-- markers are the LAST statements in the file, so a failure anywhere above
-- rolls back before the new version is ever published.
--
-- Plain `create table` (no `if not exists`): the step runs exactly once per
-- version transition, guarded by the version marker, so an object already
-- squatting on one of these names is a real defect that must fail the
-- transaction rather than be silently accepted under a published v2 marker.
--
-- `dream_link_batches` / `dream_link_members` / `dream_link_pairs` give the
-- reconsolidation link phase durable per-batch pair progress: an exhausted
-- cycle budget mid-batch leaves the remaining pairs `queued` instead of
-- dropping them, so the next cycle resumes exactly where the previous one
-- stopped (memory-reconsolidation design; astra finding 3.2).
--
-- Source ids (`session_id`, `activation_id`, `left_id`, `right_id`) are
-- durable SNAPSHOT identifiers, deliberately without live-row foreign keys:
-- deleting a session, activation, or crystal must neither erase recorded pair
-- progress nor block the deletion itself. The dreaming phase revalidates them
-- when it resumes a batch and records a skipped tombstone for anything that
-- has since gone. Internal batch references keep their foreign keys.
--
-- `term_rule_actions` is ADR 0011's audited rule-lifecycle record: actor
-- identity, prior and resulting revision ids, timestamp, and reason for every
-- transactional lifecycle transition, keyed by an idempotency key so a
-- retried request is applied at most once. Its `rule_id` reference carries no
-- cascade: an audit record outlives the row it describes.

create table dream_link_batches (
  id integer primary key,
  session_id integer not null,
  created_cycle integer not null,
  completed_cycle integer
);

create table dream_link_members (
  batch_id integer not null references dream_link_batches(id),
  activation_id integer not null unique,
  primary key (batch_id, activation_id)
);

create table dream_link_pairs (
  batch_id integer not null references dream_link_batches(id),
  left_id integer not null,
  right_id integer not null,
  status text not null default 'queued'
    check (status in ('queued', 'applied', 'skipped')),
  applied_cycle integer,
  result_json text,
  primary key (batch_id, left_id, right_id),
  check (left_id < right_id)
);

create index dream_link_batches_session_idx
  on dream_link_batches(session_id);
create index dream_link_pairs_pending_idx
  on dream_link_pairs(batch_id, status);

-- Column set + the partial idempotency index are the contract
-- `terminology.rs::Termbase::apply_action` writes and reads (the agent-tools
-- plan's audited rule lifecycle, ADR 0011): `expected_revision` is the
-- optimistic-concurrency revision the caller asserted, `request_canonical`
-- the key-sorted JSON that makes a retried request byte-identical. The
-- `rule_id` foreign key with no cascade keeps the audit row pinned to its
-- subject — an audited rule is archived/superseded, never hard-deleted.
create table term_rule_actions (
  id integer primary key,
  idempotency_key text not null default '',
  rule_id integer not null references term_rules(id),
  actor text not null,
  reason text not null,
  action text not null,
  expected_revision integer not null,
  resulting_revision integer not null,
  request_canonical text not null,
  result_json text not null,
  created_at text not null
);

create unique index term_rule_actions_idempotency_key_idx
  on term_rule_actions(idempotency_key) where idempotency_key <> '';
create index term_rule_actions_rule_idx
  on term_rule_actions(rule_id, id);

-- Version markers last: every table above exists before v2 is claimed.
update hieronymus_meta set schema_version = 2;
pragma user_version = 2;
