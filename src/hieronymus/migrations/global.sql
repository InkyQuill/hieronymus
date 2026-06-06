create table if not exists series (
  id integer primary key,
  slug text not null unique,
  title text not null,
  default_source_language text not null,
  default_target_language text not null,
  created_at text not null,
  updated_at text not null
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
  completed_at text
);

create table if not exists short_term_memories (
  id integer primary key,
  session_id integer not null references task_sessions(id) on delete cascade,
  source_role text not null,
  kind text not null,
  text text not null,
  source_ref text not null default '',
  metadata_json text not null default '{}',
  created_at text not null,
  archived_at text
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
  status text not null,
  created_cycle integer not null default 0,
  last_activated_cycle integer,
  last_reinforced_cycle integer,
  created_at text not null,
  updated_at text not null
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
