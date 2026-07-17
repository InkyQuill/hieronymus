create table if not exists series (
  id integer primary key,
  slug text not null unique,
  title text not null,
  default_source_language text not null,
  default_target_language text not null,
  created_at text not null,
  updated_at text not null
);

create table if not exists series_language_tags (
  series_id integer not null references series(id) on delete cascade,
  language_tag text not null,
  created_at text not null default (datetime('now')),
  primary key (series_id, language_tag)
);

create table if not exists task_sessions (
  id integer primary key,
  series_slug text not null references series(slug),
  source_language text not null,
  target_language text not null,
  task_type text not null,
  volume text not null default '',
  chapter text not null default '',
  status text not null,
  cycle_id integer,
  created_at text not null,
  last_activity_at text not null,
  completed_at text
);

create table if not exists task_session_language_tags (
  session_id integer not null references task_sessions(id) on delete cascade,
  language_tag text not null,
  primary key (session_id, language_tag)
);

create table if not exists task_session_story_scopes (
  session_id integer not null references task_sessions(id) on delete cascade,
  story_scope text not null,
  primary key (session_id, story_scope)
);

create table if not exists task_session_semantic_tags (
  session_id integer not null references task_sessions(id) on delete cascade,
  semantic_tag text not null,
  primary key (session_id, semantic_tag)
);

create table if not exists short_term_memories (
  id integer primary key,
  session_id integer not null references task_sessions(id) on delete cascade,
  source_role text not null,
  kind text not null,
  text text not null,
  source_ref text not null default '',
  metadata_json text not null default '{}',
  source_credibility text,
  rule_intent text,
  soft_origin text,
  created_at text not null,
  archived_at text
);

create table if not exists short_term_memory_language_tags (
  memory_id integer not null references short_term_memories(id) on delete cascade,
  language_tag text not null,
  primary key (memory_id, language_tag)
);

create table if not exists short_term_memory_story_scopes (
  memory_id integer not null references short_term_memories(id) on delete cascade,
  story_scope text not null,
  primary key (memory_id, story_scope)
);

create table if not exists short_term_memory_semantic_tags (
  memory_id integer not null references short_term_memories(id) on delete cascade,
  semantic_tag text not null,
  primary key (memory_id, semantic_tag)
);

create virtual table if not exists short_term_memories_fts using fts5(
  text,
  content='short_term_memories',
  content_rowid='id'
);

create table if not exists crystals (
  id integer primary key,
  crystal_type text not null,
  text text not null,
  title text not null default '',
  scope_type text not null,
  scope_key text not null default '',
  series_slug text not null default '',
  source_language text not null default '',
  target_language text not null default '',
  tags_json text not null default '[]',
  strength real not null,
  confidence real not null,
  source_credibility text not null default 'observation',
  rule_intent text not null default '',
  soft_origin text,
  is_inferred integer not null default 0,
  malformed_penalty real not null default 0.0,
  supersedes_crystal_id integer references crystals(id) on delete set null,
  status text not null,
  created_cycle integer not null default 0,
  last_activated_cycle integer,
  last_reinforced_cycle integer,
  created_at text not null,
  updated_at text not null
);

create table if not exists crystal_language_tags (
  crystal_id integer not null references crystals(id) on delete cascade,
  language_tag text not null,
  primary key (crystal_id, language_tag)
);

create virtual table if not exists crystals_fts using fts5(
  title,
  text,
  content='crystals',
  content_rowid='id'
);

create table if not exists crystal_sources (
  crystal_id integer not null references crystals(id) on delete cascade,
  short_term_memory_id integer not null references short_term_memories(id) on delete cascade,
  primary key(crystal_id, short_term_memory_id)
);

create table if not exists crystal_links (
  source_crystal_id integer not null references crystals(id) on delete cascade,
  target_crystal_id integer not null references crystals(id) on delete cascade,
  link_type text not null,
  primary key(source_crystal_id, target_crystal_id, link_type)
);

create table if not exists crystal_activations (
  id integer primary key,
  crystal_id integer not null references crystals(id) on delete cascade,
  session_id integer not null references task_sessions(id) on delete cascade,
  recall_query text not null,
  rank integer not null,
  score real not null,
  reason text not null default '',
  cycle_id integer,
  created_at text not null
);

create table if not exists memory_events (
  id integer primary key,
  crystal_id integer references crystals(id) on delete set null,
  session_id integer references task_sessions(id) on delete set null,
  event_type text not null,
  source_role text not null,
  evidence text not null default '',
  strength_delta real not null default 0,
  confidence_delta real not null default 0,
  applied integer not null default 0,
  cycle_id integer,
  created_at text not null
);

create table if not exists dream_runs (
  id integer primary key,
  cycle_id integer not null unique,
  status text not null,
  provider text not null,
  input_count integer not null default 0,
  created_crystal_count integer not null default 0,
  proposal_count integer not null default 0,
  error text not null default '',
  created_at text not null,
  completed_at text
);

create table if not exists strict_concept_proposals (
  id integer primary key,
  dream_run_id integer references dream_runs(id) on delete set null,
  series_slug text not null default '',
  source_language text not null,
  target_language text not null,
  concept_text text not null,
  source_form text not null,
  canonical_rendering text not null,
  approved_variants_json text not null default '[]',
  forbidden_variants_json text not null default '[]',
  rationale text not null default '',
  status text not null,
  created_at text not null,
  updated_at text not null
);

create table if not exists strict_terms (
  id integer primary key,
  series_slug text not null references series(slug),
  source_language text not null,
  target_language text not null,
  category text not null,
  source_text text not null,
  canonical_translation text not null,
  status text not null,
  notes text not null default '',
  created_at text not null,
  updated_at text not null
);

create table if not exists strict_term_tags (
  term_id integer not null references strict_terms(id) on delete cascade,
  tag text not null,
  primary key(term_id, tag)
);

create table if not exists strict_term_aliases (
  id integer primary key,
  term_id integer not null references strict_terms(id) on delete cascade,
  language text not null,
  text text not null,
  kind text not null,
  case_sensitive integer not null default 1
);

create virtual table if not exists strict_terms_fts using fts5(
  source_text,
  canonical_translation,
  notes,
  content='strict_terms',
  content_rowid='id'
);

create table if not exists audit_log (
  id integer primary key,
  actor text not null default 'admin',
  action text not null,
  entity_type text not null,
  entity_id text not null,
  note text not null default '',
  before_json text not null default '{}',
  after_json text not null default '{}',
  created_at text not null
);

create table if not exists concepts (
  id integer primary key,
  canonical_name text not null,
  description text not null default '',
  scope_type text not null default 'global',
  scope_key text not null default '',
  status text not null default 'candidate',
  confidence real not null default 0.2,
  merged_into_concept_id integer references concepts(id),
  created_at text not null,
  updated_at text not null,
  check ((scope_type = 'global' and scope_key = '') or (scope_type != 'global' and scope_key != ''))
);

create virtual table if not exists concepts_fts using fts5(
  canonical_name,
  description,
  content='concepts',
  content_rowid='id'
);

create trigger if not exists concepts_ai
after insert on concepts
begin
  insert into concepts_fts(rowid, canonical_name, description)
  values (new.id, new.canonical_name, new.description);
end;

create trigger if not exists concepts_ad
after delete on concepts
begin
  insert into concepts_fts(concepts_fts, rowid, canonical_name, description)
  values ('delete', old.id, old.canonical_name, old.description);
end;

create trigger if not exists concepts_au
after update on concepts
begin
  insert into concepts_fts(concepts_fts, rowid, canonical_name, description)
  values ('delete', old.id, old.canonical_name, old.description);
  insert into concepts_fts(rowid, canonical_name, description)
  values (new.id, new.canonical_name, new.description);
end;

create table if not exists concept_facets (
  id integer primary key,
  concept_id integer not null references concepts(id) on delete cascade,
  language text not null default '',
  facet_type text not null,
  value text not null,
  source_crystal_id integer references crystals(id) on delete set null,
  confidence real not null default 0.2,
  is_canonical integer not null default 0,
  superseded_at text,
  created_at text not null,
  updated_at text not null
);

create virtual table if not exists concept_facet_fts using fts5(
  value,
  content='concept_facets',
  content_rowid='id'
);

create trigger if not exists concept_facets_ai
after insert on concept_facets
begin
  insert into concept_facet_fts(rowid, value)
  values (new.id, new.value);
end;

create trigger if not exists concept_facets_ad
after delete on concept_facets
begin
  insert into concept_facet_fts(concept_facet_fts, rowid, value)
  values ('delete', old.id, old.value);
end;

create trigger if not exists concept_facets_au
after update on concept_facets
begin
  insert into concept_facet_fts(concept_facet_fts, rowid, value)
  values ('delete', old.id, old.value);
  insert into concept_facet_fts(rowid, value)
  values (new.id, new.value);
end;

create table if not exists concept_facet_language_tags (
  facet_id integer not null references concept_facets(id) on delete cascade,
  language_tag text not null,
  primary key (facet_id, language_tag)
);

create table if not exists concept_facet_story_scopes (
  facet_id integer not null references concept_facets(id) on delete cascade,
  story_scope text not null,
  primary key (facet_id, story_scope)
);

create table if not exists concept_facet_semantic_tags (
  facet_id integer not null references concept_facets(id) on delete cascade,
  semantic_tag text not null,
  primary key (facet_id, semantic_tag)
);

create table if not exists concept_semantic_tags (
  concept_id integer not null references concepts(id) on delete cascade,
  tag text not null,
  confidence real not null default 0.2,
  created_at text not null,
  primary key(concept_id, tag)
);

create table if not exists concept_renames (
  id integer primary key,
  concept_id integer not null references concepts(id) on delete cascade,
  old_name text not null,
  new_name text not null,
  reason text not null default '',
  dream_run_id integer references dream_runs(id) on delete set null,
  created_at text not null
);

create table if not exists crystal_concepts (
  crystal_id integer not null references crystals(id) on delete cascade,
  concept_id integer not null references concepts(id) on delete cascade,
  link_type text not null default 'mentions',
  confidence real not null default 0.2,
  created_at text not null,
  primary key(crystal_id, concept_id, link_type)
);

create table if not exists crystal_story_scopes (
  crystal_id integer not null references crystals(id) on delete cascade,
  scope text not null,
  confidence real not null default 0.2,
  created_at text not null,
  primary key(crystal_id, scope)
);

create table if not exists crystal_semantic_tags (
  crystal_id integer not null references crystals(id) on delete cascade,
  tag text not null,
  confidence real not null default 0.2,
  created_at text not null,
  primary key(crystal_id, tag)
);

create table if not exists dream_phase_runs (
  id integer primary key,
  dream_run_id integer not null references dream_runs(id) on delete cascade,
  phase text not null,
  provider_profile text not null,
  provider_type text not null,
  model text not null,
  status text not null,
  input_count integer not null default 0,
  output_count integer not null default 0,
  error text not null default '',
  prompt_hash text not null default '',
  created_at text not null,
  completed_at text
);

create table if not exists dream_audit_entries (
  id integer primary key,
  dream_run_id integer not null references dream_runs(id) on delete cascade,
  phase_run_id integer references dream_phase_runs(id) on delete set null,
  event_type text not null,
  severity text not null default 'info',
  summary text not null,
  payload_json text not null default '{}',
  created_at text not null
);

create table if not exists memory_graph_migration_ledger (
  source_table text not null,
  source_id text not null,
  target_table text not null,
  target_id integer not null,
  created_at text not null default (datetime('now')),
  primary key (source_table, source_id, target_table)
);

create table if not exists rag_sources (
  id integer primary key,
  series_slug text not null references series(slug) on delete cascade,
  source_ref text not null,
  source_type text not null,
  content_type text not null,
  checksum text not null,
  metadata_json text not null default '{}',
  created_at text not null,
  updated_at text not null,
  unique(id, series_slug),
  unique(series_slug, source_ref)
);

create table if not exists rag_chunks (
  id integer primary key,
  source_id integer not null references rag_sources(id) on delete cascade,
  series_slug text not null references series(slug) on delete cascade,
  chunk_kind text not null,
  text text not null,
  display_text text not null,
  location text not null default '',
  metadata_json text not null default '{}',
  created_at text not null,
  foreign key (source_id, series_slug) references rag_sources(id, series_slug)
    on delete cascade
);

create index if not exists rag_chunks_source_id_idx
on rag_chunks(source_id);

create index if not exists rag_chunks_series_slug_idx
on rag_chunks(series_slug);

create table if not exists rag_chunk_language_tags (
  chunk_id integer not null references rag_chunks(id) on delete cascade,
  language_tag text not null,
  primary key (chunk_id, language_tag)
);

create table if not exists rag_chunk_story_scopes (
  chunk_id integer not null references rag_chunks(id) on delete cascade,
  story_scope text not null,
  primary key (chunk_id, story_scope)
);

create table if not exists rag_chunk_semantic_tags (
  chunk_id integer not null references rag_chunks(id) on delete cascade,
  semantic_tag text not null,
  primary key (chunk_id, semantic_tag)
);

create virtual table if not exists rag_chunks_fts using fts5(
  text,
  display_text,
  location,
  content='rag_chunks',
  content_rowid='id'
);

create trigger if not exists rag_chunks_ai
after insert on rag_chunks
begin
  insert into rag_chunks_fts(rowid, text, display_text, location)
  values (new.id, new.text, new.display_text, new.location);
end;

create trigger if not exists rag_chunks_ad
after delete on rag_chunks
begin
  insert into rag_chunks_fts(rag_chunks_fts, rowid, text, display_text, location)
  values ('delete', old.id, old.text, old.display_text, old.location);
end;

create trigger if not exists rag_chunks_au
after update on rag_chunks
begin
  insert into rag_chunks_fts(rag_chunks_fts, rowid, text, display_text, location)
  values ('delete', old.id, old.text, old.display_text, old.location);
  insert into rag_chunks_fts(rowid, text, display_text, location)
  values (new.id, new.text, new.display_text, new.location);
end;
