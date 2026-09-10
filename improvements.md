# Design & Architecture Improvements for Hieronymus

This document reviews the consistency, usefulness, and overall design of the Hieronymus local-first translation memory project manager.

---

## 1. Executive Summary & Usefulness

Hieronymus implements a robust, deterministic, and agent-friendly translation memory manager. It is **highly useful** for literary translation and long-form writing workflows because:
- **Clear Separation of Memory Tiers**: It keeps a clear distinction between transient session-scoped **Short-Term Memory**, durable long-term crystallized memories (**Crystals**), and document-level **RAG**.
- **Deterministic Terminology Safeguards**: In accordance with literary requirements, strict rules and approved termbase entries (in rule crystals) take precedence over fuzzy matches and stylistic suggestions during recall, preventing hallucinated errors.
- **Asynchronous Dreaming**: The dreaming mechanism acts as an offline consolidating background daemon. Instead of slowing down live agent interactions, it runs prompts asynchronously to turn short-term snippets into rule crystals and concepts, reinforcing relevant facts, and decaying stale entries.

However, several architectural inconsistencies and potential bugs exist that should be addressed to ensure robustness, performance, and scaling.

---

## 2. Inconsistencies & Required Fixes

### 2.1. SQLite FTS Index Triggers
- **Location**: [global.sql](file:///home/inky/Development/hieronymus/src/hieronymus/migrations/global.sql)
- **Problem**: Triggers are defined to keep FTS virtual tables in sync for `concepts`, `concept_facet_fts`, and `rag_chunks_fts`. However, no triggers exist for `short_term_memories` and `crystals`. Instead, the application codebase manually handles FTS insertion in [workspace.py](file:///home/inky/Development/hieronymus/src/hieronymus/workspace.py) and [crystals.py](file:///home/inky/Development/hieronymus/src/hieronymus/crystals.py), and updates in [admin.py](file:///home/inky/Development/hieronymus/src/hieronymus/admin.py).
- **Risk**: If a task session, series, or crystal is deleted via cascade delete (e.g., when deleting a series or purges in tests), or edited directly via admin SQL queries, the corresponding FTS index entries are not deleted or updated. This leads to orphaned FTS entries, index mismatch/drift, and potential query errors.
- **Recommendation**: Rewrite the FTS update pattern to use native SQLite triggers in `global.sql` for all memory types, removing manual FTS write commands from python domain services.

```sql
-- Example triggers to add to global.sql:
create trigger if not exists crystals_ai
after insert on crystals
begin
  insert into crystals_fts(rowid, title, text)
  values (new.id, new.title, new.text);
end;

create trigger if not exists crystals_ad
after delete on crystals
begin
  insert into crystals_fts(crystals_fts, rowid, title, text)
  values ('delete', old.id, old.title, old.text);
end;

create trigger if not exists crystals_au
after update on crystals
begin
  insert into crystals_fts(crystals_fts, rowid, title, text)
  values ('delete', old.id, old.title, old.text);
  insert into crystals_fts(rowid, title, text)
  values (new.id, new.title, new.text);
end;
```

---

### 2.2. Interactive Installer TTY Blocks in Test Environments
- **Location**: [install.sh](file:///home/inky/Development/hieronymus/install.sh)
- **Problem**: The `has_tty()` function checks if `/dev/tty` is readable and writable:
  ```sh
  has_tty() {
      [ -r /dev/tty ] && [ -w /dev/tty ] && ( : </dev/tty ) 2>/dev/null && ( : >/dev/tty ) 2>/dev/null
  }
  ```
  During automated test execution (e.g. `pytest` running subprocesses), `/dev/tty` is still available from the parent execution terminal, but stdin/stdout are captured. This causes the installer to output a prompt to `/dev/tty` and wait indefinitely for user input, hanging the test run.
- **Recommendation**: Update `has_tty()` to verify that stdin and stdout are actually connected to a terminal using `[ -t 0 ] && [ -t 1 ]`:
  ```sh
  has_tty() {
      [ -t 0 ] && [ -t 1 ] && [ -r /dev/tty ] && [ -w /dev/tty ] && ( : </dev/tty ) 2>/dev/null && ( : >/dev/tty ) 2>/dev/null
  }
  ```

---

### 2.3. Terminology DB Overlap (`strict_terms` vs. Rule Crystals)
- **Location**: [termbase.py](file:///home/inky/Development/hieronymus/src/hieronymus/termbase.py) & [memory_migration.py](file:///home/inky/Development/hieronymus/src/hieronymus/memory_migration.py)
- **Problem**: The schema retains a legacy `strict_terms` table, but the termbase and validation services have transitioned to utilizing `crystals` with `crystal_type = 'rule'`. The codebase automatically migrates approved strict terms to rule crystals on startup.
- **Recommendation**: Fully deprecate the `strict_terms` database table and simplify the database design. Direct all terminology constraints and imports to write rule crystals natively, removing the migration code and the duplicate term table.

---

## 3. Core Feature Enhancements

### 3.1. Upgrading RAG to Semantic (Vector) Search
- **Location**: [rag_store.py](file:///home/inky/Development/hieronymus/src/hieronymus/rag_store.py) & [recall.py](file:///home/inky/Development/hieronymus/src/hieronymus/recall.py)
- **Problem**: RAG search is restricted to SQLite FTS5 (BM25 keyword matching). If an agent queries "main character's weapon" but the RAG document refers to "a curved scimitar", FTS5 will miss it because of the lexical difference.
- **Recommendation**:
  - Implement a vector similarity layer. Since OpenAI, Gemini, and Ollama provider clients are already configured in `dream_providers.py`, they can be used to generate embeddings.
  - Integrate a lightweight SQLite extension (like `sqlite-vec`) or a lightweight Python embedding store.
  - Implement a hybrid search in `recall.py` that merges lexical BM25 and vector cosine similarity scores to provide semantically-aware RAG.

---

### 3.2. Bounded Dreaming Batching & Scaling
- **Location**: [dreaming.py](file:///home/inky/Development/hieronymus/src/hieronymus/dreaming.py)
- **Problem**: Ambient decay is scanned across all candidates. As the database grows to thousands of crystals over multi-volume series, processing full decay passes on every dream run might cause scaling bottlenecks.
- **Recommendation**: Index memory records on `last_reinforced_cycle` or `updated_at`, and restrict decay checks to recently accessed/active candidate sets rather than full table scans.
