CREATE TABLE semantic_index_jobs (
  id INTEGER PRIMARY KEY,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (
    status IN ('pending', 'running', 'completed', 'failed', 'cancelled')
  ),
  generation_id TEXT,
  created_at TEXT NOT NULL,
  completed_at TEXT
) STRICT;

CREATE TABLE semantic_chunk_state (
  chunk_id INTEGER PRIMARY KEY REFERENCES rag_chunks(id),
  checksum TEXT NOT NULL,
  generation_id TEXT NOT NULL,
  indexed_at TEXT NOT NULL
) STRICT;
