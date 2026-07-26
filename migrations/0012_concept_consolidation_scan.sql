CREATE TABLE concept_consolidation_keys (
  concept_id INTEGER PRIMARY KEY REFERENCES concepts(id) ON DELETE CASCADE,
  scope_type TEXT NOT NULL,
  scope_key TEXT NOT NULL,
  canonical_name_key TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;

CREATE INDEX concept_consolidation_keys_lookup_idx
ON concept_consolidation_keys(scope_type, scope_key, canonical_name_key, concept_id);

CREATE TABLE concept_consolidation_scan (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  after_concept_id INTEGER NOT NULL DEFAULT 0 CHECK (after_concept_id >= 0),
  restart_required INTEGER NOT NULL DEFAULT 0 CHECK (restart_required IN (0, 1)),
  updated_at TEXT NOT NULL
) STRICT;

INSERT INTO concept_consolidation_scan(
  singleton, after_concept_id, restart_required, updated_at
) VALUES (1, 0, 0, CURRENT_TIMESTAMP);

CREATE TABLE dream_affected_crystals (
  maintenance_cycle_id INTEGER NOT NULL,
  crystal_id INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (maintenance_cycle_id, crystal_id)
) STRICT;

CREATE TRIGGER concept_consolidation_scan_reset_after_insert
AFTER INSERT ON concepts
WHEN NEW.merged_into_concept_id IS NULL
  AND NEW.status IN ('candidate', 'established')
BEGIN
  UPDATE concept_consolidation_scan
  SET restart_required = CASE WHEN after_concept_id > 0 THEN 1 ELSE restart_required END,
      updated_at = CURRENT_TIMESTAMP
  WHERE singleton = 1;
END;

CREATE TRIGGER concept_consolidation_keys_remove_ineligible
AFTER UPDATE OF canonical_name, scope_type, scope_key, status, merged_into_concept_id ON concepts
BEGIN
  DELETE FROM concept_consolidation_keys WHERE concept_id = NEW.id;
  UPDATE concept_consolidation_scan
  SET restart_required = CASE WHEN after_concept_id > 0 THEN 1 ELSE restart_required END,
      updated_at = CURRENT_TIMESTAMP
  WHERE singleton = 1;
END;

CREATE TRIGGER concept_consolidation_scan_reset_after_delete
AFTER DELETE ON concepts
BEGIN
  UPDATE concept_consolidation_scan
  SET restart_required = CASE WHEN after_concept_id > 0 THEN 1 ELSE restart_required END,
      updated_at = CURRENT_TIMESTAMP
  WHERE singleton = 1;
END;
