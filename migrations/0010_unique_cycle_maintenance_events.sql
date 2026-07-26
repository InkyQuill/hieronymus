CREATE UNIQUE INDEX ux_memory_events_cycle_decay_crystal_cycle
ON memory_events(crystal_id, cycle_id)
WHERE event_type = 'cycle_decay'
  AND crystal_id IS NOT NULL
  AND cycle_id IS NOT NULL;
