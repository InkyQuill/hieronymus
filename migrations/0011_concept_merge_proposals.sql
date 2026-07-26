CREATE TABLE concept_merge_proposals (
  id INTEGER PRIMARY KEY,
  source_concept_id INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
  target_concept_id INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
  rationale TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'rejected')),
  dream_run_id INTEGER REFERENCES dream_runs(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  CHECK (source_concept_id <> target_concept_id)
) STRICT;

CREATE UNIQUE INDEX concept_merge_proposals_pending_pair_idx
ON concept_merge_proposals(source_concept_id, target_concept_id)
WHERE status = 'pending';
