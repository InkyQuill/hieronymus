-- C8: cursor offsets need a stable crystal identity snapshot. Activation rows
-- may be deleted; these identities deliberately have no crystal foreign key.
create table dream_link_crystals (
  batch_id integer not null references dream_link_batches(id),
  member_offset integer not null,
  crystal_id integer not null,
  primary key (batch_id, member_offset),
  unique (batch_id, crystal_id)
);
-- Existing batches retain their complete materialized pair set. New batches
-- explicitly opt into lazy enumeration, including zero/singleton snapshots.
alter table dream_link_batches add column lazy_pairs integer not null default 0
  check (lazy_pairs in (0, 1));
-- Bounded reads must not sort the legacy quadratic queue or scan the full
-- terminal/audit history on every bounded cycle.
create index dream_link_pairs_queue_order_idx
  on dream_link_pairs(batch_id, status, left_id, right_id);
create index dream_link_pairs_cycle_idx on dream_link_pairs(applied_cycle);
create index dream_link_batches_open_order_idx
  on dream_link_batches(lazy_pairs, id) where completed_cycle is null;
create index dream_link_batches_completed_idx on dream_link_batches(completed_cycle);
create index dream_feedback_cycle_idx on memory_events(cycle_id)
  where event_type = 'recalled_again' and applied = 1;
create index dream_audit_run_event_idx on dream_audit_entries(dream_run_id, event_type);
create index dream_activations_pending_session_idx on crystal_activations(session_id, id)
  where outcome = 'useful' and cycle_id is null;
update hieronymus_meta set schema_version = 4;
pragma user_version = 4;
