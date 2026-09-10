-- Rust terminology authority (ADR 0011). Fresh Rust databases get these
-- tables as part of schema version 1; Python databases receive them through
-- the database-upgrade import boundary.

create table if not exists term_rules (
  id integer primary key,
  concept_id integer references concepts(id),
  source_language text not null default '',
  target_language text not null default '',
  source_text text not null,
  canonical_translation text not null,
  forbidden_variants_json text not null default '[]',
  matching_policy text not null default 'surface',
  status text not null check (status in ('candidate', 'active', 'superseded', 'archived')),
  provenance text not null default '',
  notes text not null default '',
  revision integer not null default 1,
  rule_crystal_id integer,
  created_at text not null,
  updated_at text not null
);

create table if not exists term_rule_forms (
  id integer primary key,
  rule_id integer not null references term_rules(id) on delete cascade,
  form_kind text not null check (form_kind in ('source', 'approved', 'forbidden')),
  surface text not null,
  language text not null default '',
  case_sensitive integer not null default 0
);

create table if not exists term_rule_revisions (
  id integer primary key,
  rule_id integer not null references term_rules(id) on delete cascade,
  actor text not null default '',
  reason text not null default '',
  prior_status text not null default '',
  new_status text not null default '',
  created_at text not null
);

create index if not exists term_rules_status_idx on term_rules(status);
create index if not exists term_rule_forms_rule_idx on term_rule_forms(rule_id);

create table if not exists term_rule_semantic_tags (
  rule_id integer not null references term_rules(id) on delete cascade,
  tag text not null,
  primary key (rule_id, tag)
);
create table if not exists term_rule_story_scopes (
  rule_id integer not null references term_rules(id) on delete cascade,
  story_scope text not null,
  primary key (rule_id, story_scope)
);
create table if not exists term_rule_language_tags (
  rule_id integer not null references term_rules(id) on delete cascade,
  language_tag text not null,
  primary key (rule_id, language_tag)
);
