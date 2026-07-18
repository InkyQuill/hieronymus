-- Authoritative non-FTS schema for fresh Rust databases.
-- All declared types are one of SQLite STRICT's portable storage classes.

CREATE TABLE series (
  id INTEGER PRIMARY KEY,
  slug TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL,
  default_source_language TEXT NOT NULL,
  default_target_language TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE series_language_tags (
  series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
  language_tag TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  PRIMARY KEY (series_id, language_tag)
) STRICT;

CREATE TABLE task_sessions (
  id INTEGER PRIMARY KEY,
  series_slug TEXT NOT NULL REFERENCES series(slug),
  source_language TEXT NOT NULL,
  target_language TEXT NOT NULL,
  task_type TEXT NOT NULL,
  volume TEXT NOT NULL DEFAULT '',
  chapter TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL CHECK (status IN ('active', 'completed', 'dreamed')),
  cycle_id INTEGER,
  created_at TEXT NOT NULL,
  last_activity_at TEXT NOT NULL,
  completed_at TEXT
) STRICT;

CREATE TABLE task_session_language_tags (
  session_id INTEGER NOT NULL REFERENCES task_sessions(id) ON DELETE CASCADE,
  language_tag TEXT NOT NULL,
  PRIMARY KEY (session_id, language_tag)
) STRICT;

CREATE TABLE task_session_story_scopes (
  session_id INTEGER NOT NULL REFERENCES task_sessions(id) ON DELETE CASCADE,
  story_scope TEXT NOT NULL,
  PRIMARY KEY (session_id, story_scope)
) STRICT;

CREATE TABLE task_session_semantic_tags (
  session_id INTEGER NOT NULL REFERENCES task_sessions(id) ON DELETE CASCADE,
  semantic_tag TEXT NOT NULL,
  PRIMARY KEY (session_id, semantic_tag)
) STRICT;

CREATE TABLE crystals (
  id INTEGER PRIMARY KEY,
  crystal_type TEXT NOT NULL CHECK (
    crystal_type IN ('lesson', 'rule', 'thought', 'observation', 'concept_note', 'concept', 'erudition')
  ),
  text TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '',
  scope_type TEXT NOT NULL,
  scope_key TEXT NOT NULL DEFAULT '',
  series_slug TEXT NOT NULL DEFAULT '',
  source_language TEXT NOT NULL DEFAULT '',
  target_language TEXT NOT NULL DEFAULT '',
  tags_json TEXT NOT NULL DEFAULT '[]',
  strength REAL NOT NULL CHECK (strength >= 0.0 AND strength <= 1.0),
  confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
  source_credibility TEXT NOT NULL DEFAULT 'observation',
  rule_intent TEXT NOT NULL DEFAULT '',
  soft_origin TEXT,
  is_inferred INTEGER NOT NULL DEFAULT 0 CHECK (is_inferred IN (0, 1)),
  malformed_penalty REAL NOT NULL DEFAULT 0.0 CHECK (malformed_penalty >= 0.0),
  supersedes_crystal_id INTEGER REFERENCES crystals(id) ON DELETE SET NULL,
  status TEXT NOT NULL CHECK (
    status IN ('active', 'candidate', 'archived', 'rejected', 'superseded')
  ),
  created_cycle INTEGER NOT NULL DEFAULT 0,
  last_activated_cycle INTEGER,
  last_reinforced_cycle INTEGER,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE crystal_language_tags (
  crystal_id INTEGER NOT NULL REFERENCES crystals(id) ON DELETE CASCADE,
  language_tag TEXT NOT NULL,
  PRIMARY KEY (crystal_id, language_tag)
) STRICT;

CREATE TABLE short_term_memories (
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES task_sessions(id) ON DELETE CASCADE,
  source_role TEXT NOT NULL,
  kind TEXT NOT NULL,
  text TEXT NOT NULL,
  source_ref TEXT NOT NULL DEFAULT '',
  metadata_json TEXT NOT NULL DEFAULT '{}',
  source_credibility TEXT,
  rule_intent TEXT,
  soft_origin TEXT,
  source_crystal_id INTEGER REFERENCES crystals(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL,
  archived_at TEXT
) STRICT;

CREATE TABLE short_term_memory_language_tags (
  memory_id INTEGER NOT NULL REFERENCES short_term_memories(id) ON DELETE CASCADE,
  language_tag TEXT NOT NULL,
  PRIMARY KEY (memory_id, language_tag)
) STRICT;

CREATE TABLE short_term_memory_story_scopes (
  memory_id INTEGER NOT NULL REFERENCES short_term_memories(id) ON DELETE CASCADE,
  story_scope TEXT NOT NULL,
  PRIMARY KEY (memory_id, story_scope)
) STRICT;

CREATE TABLE short_term_memory_semantic_tags (
  memory_id INTEGER NOT NULL REFERENCES short_term_memories(id) ON DELETE CASCADE,
  semantic_tag TEXT NOT NULL,
  PRIMARY KEY (memory_id, semantic_tag)
) STRICT;

CREATE TABLE crystal_sources (
  crystal_id INTEGER NOT NULL REFERENCES crystals(id) ON DELETE CASCADE,
  short_term_memory_id INTEGER NOT NULL REFERENCES short_term_memories(id) ON DELETE CASCADE,
  PRIMARY KEY (crystal_id, short_term_memory_id)
) STRICT;

CREATE TABLE crystal_links (
  source_crystal_id INTEGER NOT NULL REFERENCES crystals(id) ON DELETE CASCADE,
  target_crystal_id INTEGER NOT NULL REFERENCES crystals(id) ON DELETE CASCADE,
  link_type TEXT NOT NULL,
  PRIMARY KEY (source_crystal_id, target_crystal_id, link_type)
) STRICT;

CREATE TABLE crystal_activations (
  id INTEGER PRIMARY KEY,
  crystal_id INTEGER NOT NULL REFERENCES crystals(id) ON DELETE CASCADE,
  session_id INTEGER NOT NULL REFERENCES task_sessions(id) ON DELETE CASCADE,
  recall_query TEXT NOT NULL,
  rank INTEGER NOT NULL,
  score REAL NOT NULL,
  reason TEXT NOT NULL DEFAULT '',
  outcome TEXT CHECK (outcome IS NULL OR outcome IN ('useful', 'miss')),
  cycle_id INTEGER,
  created_at TEXT NOT NULL
) STRICT;

CREATE TABLE dream_runs (
  id INTEGER PRIMARY KEY,
  cycle_id INTEGER NOT NULL UNIQUE,
  status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'failed', 'skipped')),
  provider TEXT NOT NULL,
  input_count INTEGER NOT NULL DEFAULT 0,
  created_crystal_count INTEGER NOT NULL DEFAULT 0,
  proposal_count INTEGER NOT NULL DEFAULT 0,
  error TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  completed_at TEXT
) STRICT;

CREATE TABLE concepts (
  id INTEGER PRIMARY KEY,
  canonical_name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  scope_type TEXT NOT NULL DEFAULT 'global',
  scope_key TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'candidate' CHECK (
    status IN ('candidate', 'established', 'archived', 'merged')
  ),
  confidence REAL NOT NULL DEFAULT 0.2 CHECK (confidence >= 0.0 AND confidence <= 1.0),
  merged_into_concept_id INTEGER REFERENCES concepts(id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  CHECK (
    (scope_type = 'global' AND scope_key = '')
    OR (scope_type != 'global' AND scope_key != '')
  )
) STRICT;

CREATE TABLE concept_facets (
  id INTEGER PRIMARY KEY,
  concept_id INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
  language TEXT NOT NULL DEFAULT '',
  facet_type TEXT NOT NULL CHECK (
    facet_type IN ('name', 'rendering', 'description', 'note', 'alias', 'former_label')
  ),
  value TEXT NOT NULL,
  source_crystal_id INTEGER REFERENCES crystals(id) ON DELETE SET NULL,
  confidence REAL NOT NULL DEFAULT 0.2 CHECK (confidence >= 0.0 AND confidence <= 1.0),
  is_canonical INTEGER NOT NULL DEFAULT 0 CHECK (is_canonical IN (0, 1)),
  superseded_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE concept_facet_language_tags (
  facet_id INTEGER NOT NULL REFERENCES concept_facets(id) ON DELETE CASCADE,
  language_tag TEXT NOT NULL,
  PRIMARY KEY (facet_id, language_tag)
) STRICT;

CREATE TABLE concept_facet_story_scopes (
  facet_id INTEGER NOT NULL REFERENCES concept_facets(id) ON DELETE CASCADE,
  story_scope TEXT NOT NULL,
  PRIMARY KEY (facet_id, story_scope)
) STRICT;

CREATE TABLE concept_facet_semantic_tags (
  facet_id INTEGER NOT NULL REFERENCES concept_facets(id) ON DELETE CASCADE,
  semantic_tag TEXT NOT NULL,
  PRIMARY KEY (facet_id, semantic_tag)
) STRICT;

CREATE TABLE concept_semantic_tags (
  concept_id INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
  tag TEXT NOT NULL,
  confidence REAL NOT NULL DEFAULT 0.2 CHECK (confidence >= 0.0 AND confidence <= 1.0),
  created_at TEXT NOT NULL,
  PRIMARY KEY (concept_id, tag)
) STRICT;

CREATE TABLE concept_renames (
  id INTEGER PRIMARY KEY,
  concept_id INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
  old_name TEXT NOT NULL,
  new_name TEXT NOT NULL,
  reason TEXT NOT NULL DEFAULT '',
  dream_run_id INTEGER REFERENCES dream_runs(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL
) STRICT;

CREATE TABLE concept_proposals (
  id INTEGER PRIMARY KEY,
  dream_run_id INTEGER REFERENCES dream_runs(id) ON DELETE SET NULL,
  series_slug TEXT NOT NULL DEFAULT '',
  source_language TEXT NOT NULL,
  target_language TEXT NOT NULL,
  concept_text TEXT NOT NULL,
  source_form TEXT NOT NULL,
  canonical_rendering TEXT NOT NULL,
  approved_variants_json TEXT NOT NULL DEFAULT '[]',
  forbidden_variants_json TEXT NOT NULL DEFAULT '[]',
  rationale TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'rejected')),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE crystal_concepts (
  crystal_id INTEGER NOT NULL REFERENCES crystals(id) ON DELETE CASCADE,
  concept_id INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
  link_type TEXT NOT NULL DEFAULT 'mentions',
  confidence REAL NOT NULL DEFAULT 0.2 CHECK (confidence >= 0.0 AND confidence <= 1.0),
  created_at TEXT NOT NULL,
  PRIMARY KEY (crystal_id, concept_id, link_type)
) STRICT;

CREATE TABLE crystal_story_scopes (
  crystal_id INTEGER NOT NULL REFERENCES crystals(id) ON DELETE CASCADE,
  scope TEXT NOT NULL,
  confidence REAL NOT NULL DEFAULT 0.2 CHECK (confidence >= 0.0 AND confidence <= 1.0),
  created_at TEXT NOT NULL,
  PRIMARY KEY (crystal_id, scope)
) STRICT;

CREATE TABLE crystal_semantic_tags (
  crystal_id INTEGER NOT NULL REFERENCES crystals(id) ON DELETE CASCADE,
  tag TEXT NOT NULL,
  confidence REAL NOT NULL DEFAULT 0.2 CHECK (confidence >= 0.0 AND confidence <= 1.0),
  created_at TEXT NOT NULL,
  PRIMARY KEY (crystal_id, tag)
) STRICT;

CREATE TABLE memory_events (
  id INTEGER PRIMARY KEY,
  crystal_id INTEGER REFERENCES crystals(id) ON DELETE SET NULL,
  session_id INTEGER REFERENCES task_sessions(id) ON DELETE SET NULL,
  event_type TEXT NOT NULL,
  source_role TEXT NOT NULL,
  evidence TEXT NOT NULL DEFAULT '',
  strength_delta REAL NOT NULL DEFAULT 0,
  confidence_delta REAL NOT NULL DEFAULT 0,
  applied INTEGER NOT NULL DEFAULT 0 CHECK (applied IN (0, 1)),
  cycle_id INTEGER,
  created_at TEXT NOT NULL
) STRICT;

CREATE TABLE dream_phase_runs (
  id INTEGER PRIMARY KEY,
  dream_run_id INTEGER NOT NULL REFERENCES dream_runs(id) ON DELETE CASCADE,
  phase TEXT NOT NULL,
  provider_profile TEXT NOT NULL,
  provider_type TEXT NOT NULL,
  model TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'failed')),
  input_count INTEGER NOT NULL DEFAULT 0,
  output_count INTEGER NOT NULL DEFAULT 0,
  error TEXT NOT NULL DEFAULT '',
  prompt_hash TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  completed_at TEXT
) STRICT;

CREATE TABLE dream_audit_entries (
  id INTEGER PRIMARY KEY,
  dream_run_id INTEGER NOT NULL REFERENCES dream_runs(id) ON DELETE CASCADE,
  phase_run_id INTEGER REFERENCES dream_phase_runs(id) ON DELETE SET NULL,
  event_type TEXT NOT NULL,
  severity TEXT NOT NULL DEFAULT 'info',
  summary TEXT NOT NULL,
  payload_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL
) STRICT;

CREATE TABLE audit_log (
  id INTEGER PRIMARY KEY,
  actor TEXT NOT NULL DEFAULT 'admin',
  action TEXT NOT NULL,
  entity_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  note TEXT NOT NULL DEFAULT '',
  before_json TEXT NOT NULL DEFAULT '{}',
  after_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL
) STRICT;

CREATE TABLE migration_ledger (
  source_table TEXT NOT NULL,
  source_id TEXT NOT NULL,
  target_table TEXT NOT NULL,
  target_id INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  PRIMARY KEY (source_table, source_id, target_table)
) STRICT;

CREATE TABLE rag_sources (
  id INTEGER PRIMARY KEY,
  series_slug TEXT NOT NULL REFERENCES series(slug) ON DELETE CASCADE,
  source_ref TEXT NOT NULL,
  source_type TEXT NOT NULL,
  content_type TEXT NOT NULL,
  checksum TEXT NOT NULL,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (id, series_slug),
  UNIQUE (series_slug, source_ref)
) STRICT;

CREATE TABLE rag_chunks (
  id INTEGER PRIMARY KEY,
  source_id INTEGER NOT NULL REFERENCES rag_sources(id) ON DELETE CASCADE,
  series_slug TEXT NOT NULL REFERENCES series(slug) ON DELETE CASCADE,
  chunk_kind TEXT NOT NULL,
  text TEXT NOT NULL,
  display_text TEXT NOT NULL,
  location TEXT NOT NULL DEFAULT '',
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  FOREIGN KEY (source_id, series_slug)
    REFERENCES rag_sources(id, series_slug) ON DELETE CASCADE
) STRICT;

CREATE INDEX rag_chunks_source_id_idx ON rag_chunks(source_id);
CREATE INDEX rag_chunks_series_slug_idx ON rag_chunks(series_slug);

CREATE TABLE rag_chunk_language_tags (
  chunk_id INTEGER NOT NULL REFERENCES rag_chunks(id) ON DELETE CASCADE,
  language_tag TEXT NOT NULL,
  PRIMARY KEY (chunk_id, language_tag)
) STRICT;

CREATE TABLE rag_chunk_story_scopes (
  chunk_id INTEGER NOT NULL REFERENCES rag_chunks(id) ON DELETE CASCADE,
  story_scope TEXT NOT NULL,
  PRIMARY KEY (chunk_id, story_scope)
) STRICT;

CREATE TABLE rag_chunk_semantic_tags (
  chunk_id INTEGER NOT NULL REFERENCES rag_chunks(id) ON DELETE CASCADE,
  semantic_tag TEXT NOT NULL,
  PRIMARY KEY (chunk_id, semantic_tag)
) STRICT;
