-- Reverse graph traversal must not scan the source-first primary key.
CREATE INDEX idx_crystal_links_target
ON crystal_links(target_crystal_id, source_crystal_id, link_type);
