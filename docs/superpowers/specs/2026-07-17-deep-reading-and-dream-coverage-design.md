# Deep reading and coverage-driven dreaming

## Decision

This is a breaking alpha rearchitecture. It replaces the current single
`provider.crystallize(context, memories) -> DreamOutput` path and mixed
crystallization output. There is no compatibility adapter, configuration
migration, or preservation of old workflow names.

The goal is book-scale, auditable memory: direct source content stays in RAG,
the agent records every meaningful conclusion as short-term memory, and dreaming
extracts each durable record type in independent evidence-tracked passes.

## Read orchestration and RAG MCP contract

`hieronymus-read` is a generated agent skill, not a judgment MCP tool. It uses
one active reading session and these primitives in order:

1. Call the existing `hieronymus_rag_import(series_slug, path, source_ref,
   source_type, language_tags, story_scopes, semantic_tags)` for every source.
2. A successful existing `RagImportResult` is the durable import receipt. An
   MCP error identifies the source and conversion error. The agent can retry
   after the source changes but never converts documents itself.
3. Read/reason over source and RAG, then call
   `hieronymus_short_term_add_batch` for conclusions in that same session.
4. Complete the session only after every source has a receipt or a user-visible
   failed import.

RAG stores direct source content. Short-term memory stores only the agent's own
conclusions. Each block contains one to six sentences; the agent creates as
many blocks as necessary to cover all terms, concepts, facts, implications,
uncertainties, links, and translation-relevant details.

Format policy is exact:

- `.txt`, `.md`, `.markdown`: import unchanged.
- `.html`, `.docx`, text-bearing `.pdf`: convert to a managed Markdown artifact
  before parsing/chunking; the original stays unchanged and the receipt names
  the artifact.
- `.epub`: reject as unsupported; the agent splits it into chapters/files first.
- HTML uses a dedicated HTML-to-Markdown library; DOCX uses `mammoth`; PDF uses
  `pypdf` extraction plus a deterministic Markdown normalizer. Empty/scanned
  PDFs fail clearly. No runtime pandoc dependency is introduced.

## Batch short-term-memory MCP contract

```text
hieronymus_short_term_add_batch(
  session_id: int,
  items: list[ShortTermMemoryInput],
) -> {"memory_ids": list[int], "count": int}
```

Every item has the fields of `hieronymus_short_term_add` except `session_id`:
`source_role`, `kind`, `text`, `source_ref`, `metadata`, `language_tags`,
`story_scopes`, `semantic_tags`, `source_credibility`, `rule_intent`, and
`soft_origin`. All items belong to the supplied active session.

Before starting a write transaction, validate session state and every field with
the same rules as `add_short_term_memory`: nonempty text and independent
sentence/symbol ingestion thresholds. The initial maximum is 500 items. A bad
item or excess size rejects the whole request and creates no records. Returned
IDs preserve request order.

## Soft concepts and strict rules

Concepts are vague semantic entities. They may have only a name, description,
tags, and facets; source form, canonical rendering, aliases, and terminology
proposals are optional.

Terminology candidates and facets retain source forms, preferred renderings, and
alternate renderings as searchable advisory data. Approval of divergent variants
creates/updates soft concept data and never fails for that reason. Only rule
crystals are strict: they arise from explicit stable rules, prohibitions, or
user corrections. Their canonical rendering and forbidden variants remain the
sole inputs to deterministic validation.

## Replacement dream architecture

The current alpha workflows are removed: `crystallization`,
`concept_discovery`/`relation_discovery`, `rule_discovery`,
`consolidation_compaction`, and
`decay_reinforcement_review`/`reinforcement_compaction`.

They are replaced with seven separately configured provider passes. Each has a
profile, prompt, schema, phase-run row, request/response audit events, and its
own output limit:

1. `concepts`: concepts and facets only.
2. `terminology_candidates`: soft terminology proposals and aliases only.
3. `rule_crystals`: explicit strict rules and prohibitions only.
4. `knowledge_crystals`: independent long-term knowledge records only.
5. `relations`: links between new or existing long-term records only.
6. `reinforcement`: evidence-backed existing long-term record references only.
7. `coverage_audit`: deterministic coverage computation and optional model
   classification; it creates no knowledge records.

Knowledge crystals reuse existing crystal types: factual/world/character/
narrative material is `observation`, analysis is `thought`, style guidance is
`lesson`, sourced background is `erudition`, and `rule` is reserved for pass 3.

## Selection, limits, and duplicates

A run selects completed sessions oldest-first, preserves context groups, and
adds whole groups until `max_short_term_memories_per_run`. A session group is
never split. Scheduled dreaming handles one bounded run; manual dreaming drains
repeated bounded runs. Every pass receives the same selected IDs and groups.

The replacement configuration defaults are:

- `max_short_term_memories_per_run = 500`
- `max_records_per_pass = 500`
- `max_long_term_records_affected_per_run = 1_000`
- `max_relation_records_per_pass = 1_000`

Provider output above a pass limit is rejected before mutation. Every created
record must cite selected `source_memory_ids`; relations and reinforcement also
cite existing long-term IDs.

Deduplication is deterministic. Normalize type, title, text, concept links, and
scope/tags; an exact normalized match in the same series is not inserted again,
but its evidence is audited. Near matches are prompt context only; they merge
only through an explicit relation or supersession action. No semantic-similarity
auto-merge exists.

## Transaction, failure, reinforcement, and coverage

One selected run has one transaction boundary. Parse, schema-validate,
evidence-validate, deduplicate, and stage all pass output before one commit. If
any required pass fails, times out, or violates schema/limits, rollback all new
records, links, reinforcements, archiving, and mutations. Keep the failed run
and phase audit; return sessions to completed for retry.

After staging succeeds, each valid long-term ID named by `reinforcement` gets
one existing reinforcement event/strength update per run. Audit stores its
supporting short-memory IDs and event ID.

Coverage is computed from accepted staged records and their source IDs, never
trusted to LLM omission introspection. The final pass may classify uncovered
input, but cannot mark it covered. Any selected ID with no accepted
evidence-backed record fails the run as `coverage_incomplete`, rolls back all
changes, and is listed in audit data. Silent loss is impossible.

## UI, skill, and verification

The generated Read skill batches conclusions until all significant details are
represented. Web config exposes one workflow/profile/prompt per pass and the
four limits. Admin exposes per-pass input, output, accepted, rejected,
duplicate, reinforced, covered, and uncovered counts plus source/long-term
evidence.

Tests cover 500-item batch atomicity/order; all conversion policies; divergent
variant approval; all pass schemas/bounds/same-set inputs; duplicate behavior;
reinforcement; rollback on pass failure; deterministic coverage failure; and a
500-memory book-scale run through every pass.
