-- Partial cursor/range index for the exact bounded decay query in proposal 004 section 5.
CREATE INDEX idx_crystals_maintenance
ON crystals(id)
WHERE status IN ('active', 'candidate')
  AND NOT (crystal_type = 'rule' AND status = 'active');
