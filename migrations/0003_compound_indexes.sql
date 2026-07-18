-- Supports the bounded decay cursor in proposal 004 section 5.
CREATE INDEX idx_crystals_maintenance
ON crystals(status, crystal_type, last_reinforced_cycle, last_activated_cycle, id);
