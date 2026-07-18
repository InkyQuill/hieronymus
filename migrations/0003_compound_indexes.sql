-- Supports the exact id-cursor decay query in proposal 004 section 5.
CREATE INDEX idx_crystals_maintenance
ON crystals(id, created_cycle, last_activated_cycle, last_reinforced_cycle)
WHERE status IN ('active', 'candidate')
  AND NOT (crystal_type = 'rule' AND status = 'active');
