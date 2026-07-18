---
name: hieronymus-read
description: >
  Read source material into RAG and preserve concise agent conclusions in short-term memory.
---

# Hieronymus Read

Use for reading files, lookup, summaries, or temporary understanding. First import each source file
into project RAG with `hieronymus_rag_import`: RAG retains the source material itself for later
retrieval.

Do not copy file text or long extracts into short-term memory. Instead, after reading, record the
agent's own conclusions with `hieronymus_short_term_add_batch`: learned terminology, concepts,
important facts, implications, uncertainties, and connections to the current work. Each
short-term memory block must contain 1–6 sentences. Create as many separate blocks as necessary
to cover every important term, concept, and detail. The size limit applies to each block,
never to the total set.
Accumulate up to 500 validated blocks per request. Continue making batches until every important
detail is covered; a book commonly needs hundreds of conclusion blocks.

MCP tools are storage and retrieval primitives, not judgment engines. There is no supported Read
judgment MCP tool; use this skill workflow plus `hieronymus_short_term_add_batch`.

source_role is optional provenance metadata and does not classify the resulting crystal.

RAG stores the direct source; short-term memory stores the agent's indirect understanding of it.

Strict concept contracts are mandatory. Crystals and lessons are advisory.
Do not approve terminology proposals yourself; record proposals for human approval instead.
