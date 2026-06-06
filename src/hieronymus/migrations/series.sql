create table if not exists terms (
  id integer primary key,
  override_of_term_id integer references terms(id),
  category text not null,
  source_text text not null,
  canonical_translation text not null,
  status text not null,
  scope text not null,
  volume text,
  confidence real not null default 1.0,
  notes text not null default '',
  created_at text not null,
  updated_at text not null
);

create table if not exists term_tags (
  term_id integer not null references terms(id),
  tag text not null,
  primary key(term_id, tag)
);

create table if not exists term_aliases (
  id integer primary key,
  term_id integer not null references terms(id),
  language text not null,
  text text not null,
  kind text not null,
  case_sensitive integer not null default 1
);

create table if not exists term_evidence (
  id integer primary key,
  term_id integer not null references terms(id),
  source_type text not null,
  source_ref text not null,
  quote text not null default '',
  url text not null default '',
  notes text not null default '',
  created_at text not null
);

create table if not exists memories (
  id integer primary key,
  kind text not null,
  text text not null,
  importance integer not null default 3,
  status text not null default 'active',
  source_ref text not null default '',
  created_at text not null,
  updated_at text not null
);

create virtual table if not exists terms_fts using fts5(
  source_text,
  canonical_translation,
  notes,
  content='terms',
  content_rowid='id'
);

create virtual table if not exists memories_fts using fts5(
  text,
  content='memories',
  content_rowid='id'
);
