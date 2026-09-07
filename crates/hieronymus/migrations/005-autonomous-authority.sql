-- ADR 0016: one caller-transaction-owned v4 -> v5 step. No provider work.
-- Legacy term_rules are globally resolved in v4, even without any series.
-- Preserve that reach explicitly; never infer ownership from concept metadata.
create table authority_state (
  series_id integer primary key references series(id),
  revision integer not null default 0 check (typeof(revision) = 'integer' and revision >= 0)
);
create table origin_receipts (
  id text primary key not null,
  kind text not null check (kind in ('console_user','host_user_event','agent','dream','legacy_import')),
  principal text not null,
  session_id integer references task_sessions(id),
  event_id text not null,
  text text not null,
  context_json text not null check (json_valid(context_json)),
  content_hash text not null,
  created_at text not null,
  unique (kind, principal, event_id)
);
create table decision_records (
  decision_id text primary key not null,
  series_id integer not null references series(id),
  origin_id text not null references origin_receipts(id),
  actor_kind text not null check (actor_kind in ('explicit_user','agent','dream')),
  expected_revision integer not null check (typeof(expected_revision) = 'integer' and expected_revision >= 0),
  resulting_revision integer not null check (typeof(resulting_revision) = 'integer' and resulting_revision >= 0),
  canonical_request text not null check (json_valid(canonical_request)),
  result_json text not null check (json_valid(result_json)),
  status text not null check (status in ('applied','tentative')),
  created_at text not null
);
create index decision_records_revision_idx on decision_records(series_id, resulting_revision);
create table decision_evidence (
  decision_id text not null references decision_records(decision_id),
  ordinal integer not null check (ordinal >= 0),
  kind text not null check (kind in ('source_passage','aligned_rendering','observation','user_event')),
  source_id integer not null check (source_id > 0),
  hash text not null,
  span_start integer not null check (span_start >= 0),
  span_end integer not null check (span_end > span_start),
  primary key (decision_id, ordinal)
);
create table evidence_records (
  id integer primary key,
  series_id integer not null references series(id),
  kind text not null check (kind in ('source_passage','aligned_rendering','observation','user_event')),
  source_identity text not null,
  source_hash text not null,
  span_start integer not null check (span_start >= 0),
  span_end integer not null check (span_end > span_start),
  content text not null,
  binding_json text not null check (json_valid(binding_json)),
  created_at text not null,
  unique (kind, source_identity, source_hash, span_start, span_end)
);
create table story_timelines (
  id integer primary key,
  series_id integer not null references series(id),
  name text not null,
  revision integer not null default 0 check (typeof(revision) = 'integer' and revision >= 0),
  unique (series_id, name)
);
create table story_positions (
  id integer primary key,
  timeline_id integer not null references story_timelines(id),
  volume_key text not null,
  chapter_key text not null,
  scene_key text not null default '',
  ordinal integer not null check (typeof(ordinal) = 'integer' and ordinal >= 0),
  evidence_id integer not null references evidence_records(id),
  unique (timeline_id, ordinal),
  unique (timeline_id, volume_key, chapter_key, scene_key)
);
create table applicabilities (
  id integer primary key,
  series_id integer references series(id),
  timeline_id integer references story_timelines(id),
  volume_key text,
  chapter_key text,
  scope_predicates_json text not null check (json_valid(scope_predicates_json)),
  valid_from integer references story_positions(id),
  valid_until integer references story_positions(id),
  metadata_state text not null check (metadata_state in ('resolved','unspecified','legacy_global')),
  check ((series_id is null) = (metadata_state = 'legacy_global')),
  check (metadata_state != 'legacy_global' or
    (timeline_id is null and volume_key is null and chapter_key is null and
     valid_from is null and valid_until is null and scope_predicates_json = '[]'))
);
create table knowledge_gates (
  id integer primary key,
  applicability_id integer not null references applicabilities(id),
  viewpoint_kind text not null check (viewpoint_kind in ('narrator','character','all')),
  viewpoint_concept_id integer references concepts(id),
  known_from integer references story_positions(id),
  known_until integer references story_positions(id),
  check ((viewpoint_kind = 'character') = (viewpoint_concept_id is not null))
);
create table memory_claims (
  id integer primary key,
  series_id integer not null references series(id),
  concept_id integer references concepts(id),
  text text not null,
  revision integer not null default 0 check (typeof(revision) = 'integer' and revision >= 0),
  status text not null check (status in ('current','invalid','qualified','tentative')),
  qualification text,
  applicability_id integer not null references applicabilities(id),
  evolves_from integer references memory_claims(id),
  created_at text not null,
  updated_at text not null
);
create table claim_bindings (
  id integer primary key,
  claim_id integer not null references memory_claims(id),
  short_term_id integer references short_term_memories(id),
  crystal_id integer references crystals(id),
  facet_id integer references concept_facets(id),
  rag_chunk_id integer references rag_chunks(id),
  check ((short_term_id is not null) + (crystal_id is not null) +
         (facet_id is not null) + (rag_chunk_id is not null) = 1),
  unique (claim_id, short_term_id),
  unique (claim_id, crystal_id),
  unique (claim_id, facet_id),
  unique (claim_id, rag_chunk_id)
);
create table claim_effects (
  id integer primary key,
  claim_id integer not null references memory_claims(id),
  decision_id text not null references decision_records(decision_id),
  applicability_id integer not null references applicabilities(id),
  effect text not null check (effect in ('invalid','qualified')),
  qualification text,
  supersedes_effect_id integer references claim_effects(id)
);
create index claim_effects_claim_idx on claim_effects(claim_id);
create table rule_authority (
  rule_id integer primary key references term_rules(id),
  authority text not null check (authority in ('learned','explicit_user')),
  origin_id text not null references origin_receipts(id),
  decision_id text references decision_records(decision_id),
  consolidation_result_id text references consolidation_results(result_id),
  applicability_id integer not null references applicabilities(id),
  legacy_protected integer not null check (legacy_protected in (0,1)),
  check ((decision_id is null) or (consolidation_result_id is null)),
  check (legacy_protected = 0 or authority = 'explicit_user')
);
create table rule_exclusions (
  id integer primary key,
  rule_id integer not null references term_rules(id),
  applicability_id integer not null references applicabilities(id),
  decision_id text references decision_records(decision_id),
  consolidation_result_id text references consolidation_results(result_id),
  check ((decision_id is not null) + (consolidation_result_id is not null) = 1)
);
create table claim_derivations (
  input_claim_id integer not null references memory_claims(id),
  output_claim_id integer not null references memory_claims(id),
  consolidation_result_id text not null references consolidation_results(result_id),
  primary key (input_claim_id, output_claim_id, consolidation_result_id),
  check (input_claim_id != output_claim_id)
);
create table provider_recovery_state (
  provider_slot_id text primary key not null,
  config_fingerprint text not null,
  next_recovery_at text not null,
  updated_at text not null
);
create table consolidation_results (
  result_id text primary key not null,
  job_decision_id text not null references consolidation_jobs(decision_id),
  generation integer not null check (typeof(generation) = 'integer' and generation >= 0),
  state text not null check (state in ('reserved','prepared','stale','complete')),
  expected_revision integer check (expected_revision is null or (typeof(expected_revision) = 'integer' and expected_revision >= 0)),
  canonical_output text check (canonical_output is null or json_valid(canonical_output)),
  origin_id text references origin_receipts(id),
  completion_receipt text check (completion_receipt is null or json_valid(completion_receipt)),
  created_at text not null,
  updated_at text not null,
  unique (job_decision_id, generation),
  check (state = 'reserved' or (canonical_output is not null and expected_revision is not null and origin_id is not null)),
  check (state != 'complete' or completion_receipt is not null)
);
create table consolidation_jobs (
  decision_id text primary key not null references decision_records(decision_id),
  state text not null check (state in ('pending','leased','retry','degraded','failed','complete')),
  attempts integer not null default 0 check (typeof(attempts) = 'integer' and attempts >= 0),
  next_attempt_at text,
  lease_until text,
  lease_token text,
  last_error_code text,
  provider_slot_id text not null references provider_recovery_state(provider_slot_id),
  result_generation integer not null default 0 check (typeof(result_generation) = 'integer' and result_generation >= 0),
  last_attempt_at text,
  created_at text not null,
  updated_at text not null
);
create index consolidation_jobs_due_idx on consolidation_jobs(state, next_attempt_at);

insert into authority_state(series_id) select id from series;
-- Fixed receipt content is migration policy, not a claim of human authorship.
-- content_hash is SHA-256(text || LF || context_json), fixed canonical bytes.
insert into origin_receipts(id,kind,principal,event_id,text,context_json,content_hash,created_at)
select '00000000-0000-5000-8000-000000000005', 'legacy_import', 'schema-upgrade-v5',
  'v4-rule-backfill', 'Preserve legacy rule authority; provenance does not prove user authorship.',
  '{"policy":"v5-legacy-global"}',
  '3e2f6671e9b2a3c174cf099f00867ac4e6f93b0351e9af471896af508b86a26c', strftime('%Y-%m-%dT%H:%M:%fZ','now')
where exists (select 1 from term_rules);
insert into applicabilities(id,scope_predicates_json,metadata_state)
select id, '[]', 'legacy_global' from term_rules;
insert into rule_authority(rule_id,authority,origin_id,applicability_id,legacy_protected)
select id, case when status = 'candidate' then 'learned' else 'explicit_user' end,
  '00000000-0000-5000-8000-000000000005', id,
  case when status = 'candidate' then 0 else 1 end
from term_rules;

-- Assertions execute before markers. Temporary bookkeeping never becomes
-- runtime schema; a rejected statement leaves the caller to roll back.
create temp table authority_v5_backfill_check (
  ok integer not null check (ok = 1)
);
insert into authority_v5_backfill_check values (
  (select count(*) from rule_authority) = (select count(*) from term_rules)
  and (select count(*) from authority_state) = (select count(*) from series)
  and not exists (select 1 from pragma_foreign_key_check)
);
drop table authority_v5_backfill_check;

-- Only the migration may create the legacy-global exception or ownerless
-- authority. Reusing a legacy applicability for a new decision is also denied.
create trigger applicabilities_no_new_legacy before insert on applicabilities
when new.metadata_state = 'legacy_global'
begin select raise(abort, 'legacy-global applicability is migration-owned'); end;
create trigger applicabilities_no_legacy_update before update on applicabilities
when old.metadata_state = 'legacy_global' or new.metadata_state = 'legacy_global'
begin select raise(abort, 'legacy-global applicability is immutable'); end;
create trigger rule_authority_insert_guard before insert on rule_authority
when (new.decision_id is null and new.consolidation_result_id is null)
  or new.legacy_protected != 0
  or exists (select 1 from applicabilities where id = new.applicability_id and metadata_state = 'legacy_global')
begin select raise(abort, 'new authority requires an owner and series applicability'); end;
create trigger rule_authority_update_guard before update on rule_authority
when (new.decision_id is null and new.consolidation_result_id is null)
  or new.legacy_protected != 0
  or exists (select 1 from applicabilities where id = new.applicability_id and metadata_state = 'legacy_global')
begin select raise(abort, 'new authority requires an owner and series applicability'); end;
create trigger origin_receipts_immutable_update before update on origin_receipts
begin select raise(abort, 'origin receipts are immutable'); end;
create trigger origin_receipts_immutable_delete before delete on origin_receipts
begin select raise(abort, 'origin receipts are retained'); end;
create trigger evidence_records_immutable_update before update on evidence_records
begin select raise(abort, 'evidence records are immutable'); end;
create trigger evidence_records_immutable_delete before delete on evidence_records
begin select raise(abort, 'evidence records are retained'); end;

update hieronymus_meta set schema_version = 5;
pragma user_version = 5;
