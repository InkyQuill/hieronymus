create index if not exists idx_crystals_dream_maintenance on crystals(
  id,
  last_activated_cycle,
  last_reinforced_cycle,
  created_cycle
) where status = 'candidate' or (status = 'active' and crystal_type != 'rule');
