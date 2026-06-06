create table if not exists series (
  id integer primary key,
  slug text unique not null,
  title text not null,
  source_language text not null,
  target_language text not null,
  database_path text not null,
  created_at text not null,
  updated_at text not null
);
