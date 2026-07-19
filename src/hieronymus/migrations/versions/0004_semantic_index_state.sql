create table if not exists semantic_index_state (
  singleton integer primary key check (singleton = 1),
  active_generation text,
  desired_provider text,
  desired_model text,
  desired_revision text,
  desired_dimensions integer,
  updated_at text not null
);

create table if not exists semantic_index_jobs (
  id integer primary key,
  generation text not null unique,
  provider text not null,
  model text not null,
  revision text,
  dimensions integer not null,
  previous_generation text,
  manifest_checksum text not null,
  status text not null check (status in ('pending', 'running', 'completed', 'failed', 'cancelled')),
  expected_count integer not null,
  processed_count integer not null default 0,
  error text not null default '',
  retryable integer not null default 0 check (retryable in (0, 1)),
  cancel_requested integer not null default 0 check (cancel_requested in (0, 1)),
  index_started integer not null default 0 check (index_started in (0, 1)),
  created_at text not null,
  updated_at text not null
);

create index if not exists semantic_index_jobs_status_idx
on semantic_index_jobs(status, id);

create table if not exists semantic_index_job_items (
  job_id integer not null references semantic_index_jobs(id) on delete cascade,
  chunk_id integer not null,
  operation text not null check (operation in ('upsert', 'delete')),
  checksum text,
  processed integer not null default 0 check (processed in (0, 1)),
  primary key (job_id, operation, chunk_id)
);

create index if not exists semantic_index_job_items_pending_idx
on semantic_index_job_items(job_id, processed, chunk_id, operation);

create table if not exists semantic_indexed_chunks (
  generation text not null,
  chunk_id integer not null,
  checksum text not null,
  primary key (generation, chunk_id)
);
