# RAG Pipelines Design

## Context

Hieronymus already has a memory-retrieval path built around short-term memories,
long-term crystals, concepts, facets, rule crystals, and dreaming. That path is
good for durable translation memory, approved terminology, and learned workflow
lessons, but it is not the right place to store every source chapter, translator
note, or glossary row as a crystal.

The new RAG pipeline adds a separate evidence retrieval lane for project texts
and glossaries. It must improve recall context without weakening the existing
rule-crystal guarantees:

- Active rule crystals remain mandatory and protected.
- Fuzzy memory and RAG evidence remain advisory.
- Raw project text can support translation decisions, but it cannot silently
  override approved terminology or rules.
- The first implementation uses SQLite FTS5. Embeddings are required later and
  must be accounted for in the schema and APIs, but they are not part of this
  MVP.

## Goals

- Import explicit `.txt`, `.md`, `.json`, `.yaml`, `.yml`, `.csv`, and `.tsv`
  files as project RAG sources.
- Store source metadata, checksums, and searchable chunks in the existing local
  SQLite database.
- Search RAG chunks independently from memory crystals and short-term memory.
- Merge memory and RAG recall through a budgeted dual-lane recall strategy.
- Preserve explainability by returning source references, chunk kind, location,
  score, and rank reason.
- Keep the design ready for a later optional embedding index without introducing
  provider/model dependencies now.

## Non-Goals

- No directory crawling or include/exclude rules in the MVP.
- No embedding generation or vector search in the MVP.
- No LLM-based proposition chunking, contextual compression, or provider-backed
  reranking in the MVP.
- No automatic promotion of RAG evidence into rule crystals.
- No replacement of current crystal, concept, termbase, or dreaming logic.

## Architecture

The implementation adds a separate RAG storage and retrieval layer:

- `rag_sources` stores one imported file or logical glossary source.
- `rag_chunks` stores searchable chunks derived from a source.
- `rag_chunks_fts` indexes chunk text and display text with SQLite FTS5.
- `RagStore` owns import, checksum comparison, source replacement, deletion,
  and search.
- `RecallService` calls both the existing memory lane and the new RAG lane, then
  merges results.
- `RecallResult` gains a third source type, `rag`, with a RAG chunk payload.

The current memory lane remains responsible for short-term memory, crystals,
concept boosts, metadata boosts, activation recording, and rule-crystal
protection. RAG results do not write to `crystal_activations`.

## Data Model

`rag_sources`:

- `id`
- `series_slug`
- `source_ref`
- `source_type`: `text`, `markdown`, or `glossary`
- `content_type`: detected or user-provided file type such as `txt`, `md`,
  `json`, `yaml`, `csv`, or `tsv`
- `checksum`
- `metadata_json`
- `created_at`
- `updated_at`

`rag_chunks`:

- `id`
- `source_id`
- `series_slug`
- `chunk_kind`: `text`, `markdown_section`, or `glossary_entry`
- `text`
- `display_text`
- `location`
- `metadata_json`
- `created_at`

RAG chunk metadata uses side tables to match existing memory conventions:

- `rag_chunk_language_tags`
- `rag_chunk_story_scopes`
- `rag_chunk_semantic_tags`

`rag_chunks_fts` indexes `text`, `display_text`, and `location`. The chunk IDs
must be stable enough within a source replacement transaction to support later
embedding records. A future `rag_embeddings` side table can use `chunk_id`,
model/provider identity, vector dimension, checksum, and index version.

## Ingestion

The MVP indexes only explicit files passed to a CLI or MCP command.

Plain text and Markdown:

- `.txt` is treated as plain text.
- `.md` is treated as Markdown but parsed conservatively.
- Chunking follows paragraphs and headings, with a maximum character cap.
- Oversized paragraphs are split on sentence or word boundaries.
- Markdown headings become chunk location context when available.

Glossaries:

- `.csv` and `.tsv` treat each row as one glossary entry.
- Headers become field names.
- `.json` accepts a list of objects or a dictionary of entries.
- `.yaml` and `.yml` follow the same structure as JSON.
- Each glossary entry remains one chunk so terms, renderings, aliases, notes,
  category, and scope stay together.

Import is idempotent by `series_slug + source_ref + checksum`:

- If the source exists and the checksum is unchanged, import is a no-op.
- If the source exists and the checksum changed, old chunks are replaced
  transactionally.
- If parsing fails, the old indexed version remains available.
- If the extension is unsupported or the file is missing, import fails with a
  clear validation error.

## Retrieval

RAG search is scoped by `series_slug` and uses FTS5 for the MVP.

RAG scoring combines:

- FTS rank.
- Story-scope boost.
- Language-tag boost.
- Semantic-tag boost.
- Glossary-entry boost.

The exact weights can start conservative and mirror the existing recall style:
metadata and glossary boosts should help tie-breaking and obvious relevance, not
make weak textual matches outrank strong exact matches.

Rank reasons should distinguish at least:

- `rag project text match`
- `rag markdown section match`
- `rag glossary match`

## Dual-Lane Recall Merge

Recall consists of:

1. Protected relevant active rule crystals.
2. The existing memory lane: short-term memory, crystals, concepts, and metadata.
3. The RAG lane: project text and glossary chunks.

The merge is budgeted, not a hard split. For `limit=10`, the default target is
roughly `5` memory results and `5` RAG results after protected rule crystals.
If one lane has fewer results than its quota, the other lane can fill the open
slots. This avoids losing useful context when a project has no RAG corpus yet or
when a query only matches one lane.

The final list should interleave memory and RAG results after protected rules so
agents see a mixed context. It should not simply append all RAG results after all
memory results.

RAG evidence never creates an active rule by itself. If a repeated RAG finding
should become memory, the existing short-term memory and dreaming path remains
the promotion mechanism.

## Interfaces

CLI:

- `hiero rag import <series> <path> --type auto|text|glossary --source-ref <ref>`
- `hiero rag search <series> <query> --limit 10`

MCP interfaces are part of the MVP:

- `hieronymus_rag_import`
- `hieronymus_rag_search`

Admin TUI screens for RAG source management are backlog work, not MVP work.

Existing recall interfaces keep working. Their enriched payload adds RAG entries
with enough fields for citation:

- `tier` or `source`: `rag`
- `id`
- `title`
- `kind`
- `text`
- `source_ref`
- `chunk_kind`
- `location`
- `language_tags`
- `story_scopes`
- `semantic_tags`
- `score`
- `rank_reason`

## Errors

- Unsupported extension: fail before writing any source/chunk rows.
- Missing file: fail before writing any rows.
- Malformed JSON/YAML/CSV/TSV: fail import and preserve any previous indexed
  version.
- Empty file or file with no usable chunks: record a clear no-content error.
- Recall with no RAG corpus: behave like current recall.
- RAG search with no matching chunks: return an empty RAG result list.

## Testing

Unit and integration coverage should include:

- RAG migration creates tables and FTS indexes idempotently.
- Text import creates stable searchable chunks.
- Markdown import preserves heading/location context.
- CSV and TSV rows become glossary chunks.
- JSON glossary entries become glossary chunks.
- Unchanged checksum skips reindexing.
- Changed checksum replaces chunks transactionally.
- Failed import preserves the old indexed source.
- RAG search returns source references and locations.
- Combined recall returns mixed memory and RAG results.
- Active rule crystals rank above RAG evidence.
- Budgeted merge fills empty quotas from the other lane.
- Enriched payloads include fields needed for agent citation.

## Backlog

- Embedding/vector index for semantic RAG retrieval.
- Provider/model configuration for embedding generation.
- Embedding reindex lifecycle keyed by chunk checksum, model identity, and index
  version.
- RAG activation/audit table.
- Directory import with include/exclude rules.
- Admin TUI screens for RAG source management.
- Richer Markdown parser.
- LLM proposition chunking.
- Contextual compression.
- Provider-backed reranking.
- Query transformations such as step-back queries or sub-query decomposition.
- Fusion retrieval across FTS5 and embeddings.
