create index if not exists idx_crystals_dream_maintenance on crystals(
  max(
    coalesce(last_reinforced_cycle, -1),
    coalesce(last_activated_cycle, -1),
    coalesce(created_cycle, -1)
  ),
  id
) where status = 'candidate' or (status = 'active' and crystal_type != 'rule');
