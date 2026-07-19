CREATE TABLE semantic_batch_claims (
  claim_token TEXT NOT NULL,
  job_id INTEGER NOT NULL REFERENCES semantic_index_jobs(id) ON DELETE CASCADE,
  chunk_id INTEGER NOT NULL REFERENCES rag_chunks(id) ON DELETE CASCADE,
  generation_id TEXT,
  created_at TEXT NOT NULL,
  PRIMARY KEY (claim_token, chunk_id),
  UNIQUE (job_id, chunk_id)
) STRICT;

CREATE INDEX semantic_batch_claims_job_token_idx
  ON semantic_batch_claims(job_id, claim_token);
