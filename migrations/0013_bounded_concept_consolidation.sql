ALTER TABLE concept_consolidation_keys
ADD COLUMN status_rank INTEGER NOT NULL DEFAULT 1 CHECK (status_rank IN (0, 1));

ALTER TABLE concept_consolidation_keys
ADD COLUMN confidence REAL NOT NULL DEFAULT 0.0 CHECK (confidence >= 0.0 AND confidence <= 1.0);

CREATE INDEX concept_consolidation_target_idx
ON concept_consolidation_keys(
  scope_type,
  scope_key,
  canonical_name_key,
  status_rank,
  confidence DESC,
  concept_id
);

CREATE TABLE concept_consolidation_dirty (
  queue_id INTEGER PRIMARY KEY AUTOINCREMENT,
  concept_id INTEGER NOT NULL UNIQUE REFERENCES concepts(id) ON UPDATE CASCADE ON DELETE CASCADE,
  scope_type TEXT NOT NULL,
  scope_key TEXT NOT NULL,
  canonical_name TEXT NOT NULL,
  created_at TEXT NOT NULL
) STRICT;

INSERT INTO concept_consolidation_dirty(
  concept_id, scope_type, scope_key, canonical_name, created_at
)
SELECT id, scope_type, scope_key, canonical_name, CURRENT_TIMESTAMP
FROM concepts ORDER BY id;

CREATE TABLE concept_consolidation_emission (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  has_after_key INTEGER NOT NULL DEFAULT 0 CHECK (has_after_key IN (0, 1)),
  after_scope_type TEXT NOT NULL DEFAULT '',
  after_scope_key TEXT NOT NULL DEFAULT '',
  after_canonical_name_key TEXT NOT NULL DEFAULT '',
  active_scope_type TEXT,
  active_scope_key TEXT,
  active_canonical_name_key TEXT,
  source_after_concept_id INTEGER NOT NULL DEFAULT 0 CHECK (source_after_concept_id >= 0),
  updated_at TEXT NOT NULL
) STRICT;

INSERT INTO concept_consolidation_emission(singleton, updated_at)
VALUES (1, CURRENT_TIMESTAMP);

DROP TRIGGER concept_consolidation_scan_reset_after_insert;
DROP TRIGGER concept_consolidation_keys_remove_ineligible;
DROP TRIGGER concept_consolidation_scan_reset_after_delete;

CREATE TRIGGER concept_consolidation_dirty_after_insert
AFTER INSERT ON concepts
BEGIN
  INSERT INTO concept_consolidation_dirty(
    concept_id, scope_type, scope_key, canonical_name, created_at
  ) VALUES (
    NEW.id, NEW.scope_type, NEW.scope_key, NEW.canonical_name, CURRENT_TIMESTAMP
  )
  ON CONFLICT(concept_id) DO UPDATE SET
    scope_type=excluded.scope_type,
    scope_key=excluded.scope_key,
    canonical_name=excluded.canonical_name;
END;

CREATE TRIGGER concept_consolidation_dirty_after_update
AFTER UPDATE OF canonical_name, scope_type, scope_key, status, merged_into_concept_id, confidence
ON concepts
BEGIN
  INSERT INTO concept_consolidation_dirty(
    concept_id, scope_type, scope_key, canonical_name, created_at
  ) VALUES (
    NEW.id, NEW.scope_type, NEW.scope_key, NEW.canonical_name, CURRENT_TIMESTAMP
  )
  ON CONFLICT(concept_id) DO UPDATE SET
    scope_type=excluded.scope_type,
    scope_key=excluded.scope_key,
    canonical_name=excluded.canonical_name;
END;
