DROP INDEX idx_crystals_maintenance;

CREATE INDEX idx_crystals_maintenance
ON crystals(id)
WHERE status IN ('active', 'candidate');
