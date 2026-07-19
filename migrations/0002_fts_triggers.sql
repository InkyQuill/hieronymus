-- Search indexes are derived external-content FTS5 tables. Base-table
-- triggers are their sole write path; stores must never dual-write them.

CREATE VIRTUAL TABLE crystals_fts USING fts5(
  title,
  text,
  content='crystals',
  content_rowid='id'
);

CREATE TRIGGER crystals_ai
AFTER INSERT ON crystals
BEGIN
  INSERT INTO crystals_fts(rowid, title, text)
  VALUES (new.id, new.title, new.text);
END;

CREATE TRIGGER crystals_ad
AFTER DELETE ON crystals
BEGIN
  INSERT INTO crystals_fts(crystals_fts, rowid, title, text)
  VALUES ('delete', old.id, old.title, old.text);
END;

CREATE TRIGGER crystals_au
AFTER UPDATE OF title, text ON crystals
BEGIN
  INSERT INTO crystals_fts(crystals_fts, rowid, title, text)
  VALUES ('delete', old.id, old.title, old.text);
  INSERT INTO crystals_fts(rowid, title, text)
  VALUES (new.id, new.title, new.text);
END;

CREATE VIRTUAL TABLE short_term_memories_fts USING fts5(
  text,
  content='short_term_memories',
  content_rowid='id'
);

CREATE TRIGGER short_term_memories_ai
AFTER INSERT ON short_term_memories
BEGIN
  INSERT INTO short_term_memories_fts(rowid, text)
  VALUES (new.id, new.text);
END;

CREATE TRIGGER short_term_memories_ad
AFTER DELETE ON short_term_memories
BEGIN
  INSERT INTO short_term_memories_fts(short_term_memories_fts, rowid, text)
  VALUES ('delete', old.id, old.text);
END;

CREATE TRIGGER short_term_memories_au
AFTER UPDATE OF text ON short_term_memories
BEGIN
  INSERT INTO short_term_memories_fts(short_term_memories_fts, rowid, text)
  VALUES ('delete', old.id, old.text);
  INSERT INTO short_term_memories_fts(rowid, text)
  VALUES (new.id, new.text);
END;

CREATE VIRTUAL TABLE concepts_fts USING fts5(
  canonical_name,
  description,
  content='concepts',
  content_rowid='id'
);

CREATE TRIGGER concepts_ai
AFTER INSERT ON concepts
BEGIN
  INSERT INTO concepts_fts(rowid, canonical_name, description)
  VALUES (new.id, new.canonical_name, new.description);
END;

CREATE TRIGGER concepts_ad
AFTER DELETE ON concepts
BEGIN
  INSERT INTO concepts_fts(concepts_fts, rowid, canonical_name, description)
  VALUES ('delete', old.id, old.canonical_name, old.description);
END;

CREATE TRIGGER concepts_au
AFTER UPDATE OF canonical_name, description ON concepts
BEGIN
  INSERT INTO concepts_fts(concepts_fts, rowid, canonical_name, description)
  VALUES ('delete', old.id, old.canonical_name, old.description);
  INSERT INTO concepts_fts(rowid, canonical_name, description)
  VALUES (new.id, new.canonical_name, new.description);
END;

CREATE VIRTUAL TABLE concept_facet_fts USING fts5(
  value,
  content='concept_facets',
  content_rowid='id'
);

CREATE TRIGGER concept_facets_ai
AFTER INSERT ON concept_facets
BEGIN
  INSERT INTO concept_facet_fts(rowid, value)
  VALUES (new.id, new.value);
END;

CREATE TRIGGER concept_facets_ad
AFTER DELETE ON concept_facets
BEGIN
  INSERT INTO concept_facet_fts(concept_facet_fts, rowid, value)
  VALUES ('delete', old.id, old.value);
END;

CREATE TRIGGER concept_facets_au
AFTER UPDATE OF value ON concept_facets
BEGIN
  INSERT INTO concept_facet_fts(concept_facet_fts, rowid, value)
  VALUES ('delete', old.id, old.value);
  INSERT INTO concept_facet_fts(rowid, value)
  VALUES (new.id, new.value);
END;

CREATE VIRTUAL TABLE rag_chunks_fts USING fts5(
  text,
  display_text,
  location,
  content='rag_chunks',
  content_rowid='id'
);

CREATE TRIGGER rag_chunks_ai
AFTER INSERT ON rag_chunks
BEGIN
  INSERT INTO rag_chunks_fts(rowid, text, display_text, location)
  VALUES (new.id, new.text, new.display_text, new.location);
END;

CREATE TRIGGER rag_chunks_ad
AFTER DELETE ON rag_chunks
BEGIN
  INSERT INTO rag_chunks_fts(rag_chunks_fts, rowid, text, display_text, location)
  VALUES ('delete', old.id, old.text, old.display_text, old.location);
END;

CREATE TRIGGER rag_chunks_au
AFTER UPDATE OF text, display_text, location ON rag_chunks
BEGIN
  INSERT INTO rag_chunks_fts(rag_chunks_fts, rowid, text, display_text, location)
  VALUES ('delete', old.id, old.text, old.display_text, old.location);
  INSERT INTO rag_chunks_fts(rowid, text, display_text, location)
  VALUES (new.id, new.text, new.display_text, new.location);
END;

-- A database may already contain authoritative rows when version 2 is applied
-- after versions 3 and 4. Rebuild every index before recording the migration.
INSERT INTO crystals_fts(crystals_fts) VALUES ('rebuild');
INSERT INTO short_term_memories_fts(short_term_memories_fts) VALUES ('rebuild');
INSERT INTO concepts_fts(concepts_fts) VALUES ('rebuild');
INSERT INTO concept_facet_fts(concept_facet_fts) VALUES ('rebuild');
INSERT INTO rag_chunks_fts(rag_chunks_fts) VALUES ('rebuild');
