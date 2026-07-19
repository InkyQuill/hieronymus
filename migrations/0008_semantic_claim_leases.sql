ALTER TABLE semantic_batch_claims
  ADD COLUMN lease_expires_at TEXT;

ALTER TABLE semantic_batch_claims
  ADD COLUMN attempts INTEGER NOT NULL DEFAULT 1 CHECK (attempts >= 1);

ALTER TABLE semantic_index_jobs
  ADD COLUMN last_error TEXT NOT NULL DEFAULT '';

ALTER TABLE semantic_index_jobs
  ADD COLUMN failed_at TEXT;

CREATE INDEX semantic_batch_claims_lease_idx
  ON semantic_batch_claims(job_id, lease_expires_at, chunk_id);
