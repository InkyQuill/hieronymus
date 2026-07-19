-- Data conversion is deliberately performed by Rust between migrations 0004 and 0005.
-- SQL cannot decode and validate the structured legacy rows or prove target parity.
-- These guards make this final schema migration safe for fresh, empty, and already-converted DBs.
DROP TRIGGER IF EXISTS strict_terms_ai;
DROP TRIGGER IF EXISTS strict_terms_ad;
DROP TRIGGER IF EXISTS strict_terms_au;
DROP TABLE IF EXISTS strict_terms_fts;
DROP TABLE IF EXISTS strict_term_aliases;
DROP TABLE IF EXISTS strict_term_tags;
DROP TABLE IF EXISTS strict_terms;
