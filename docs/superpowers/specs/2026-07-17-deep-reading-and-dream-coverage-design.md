# Deep reading, normalized RAG, and coverage-driven dreaming

## Goal

Make book-scale reading produce a complete, inspectable memory substrate: source
material is retained in RAG, the agent records all significant conclusions as
compact short-term memories efficiently, and dreaming processes those memories
through dedicated passes rather than a single mixed request.

## Reading and RAG ingestion

Read is a two-lane workflow.

1. The agent sends every source file to Hieronymus RAG ingestion before deriving
   memory from it.
2. RAG retains direct source content; short-term memory never duplicates source
   paragraphs or long extracts.
3. The agent emits its own conclusions as short-term memory blocks of one to six
   sentences. It creates as many blocks as needed to cover every meaningful term,
   concept, fact, implication, uncertainty, connection, and translation-relevant
   detail. The per-block length is not a total memory limit.

Input format policy is deterministic:

- `.txt`, `.md`, and `.markdown` import unchanged.
- `.html`, `.docx`, and text-bearing `.pdf` are converted by Hieronymus to
  Markdown before RAG parsing and storage.
- `.epub` is unsupported by automatic ingestion. The agent splits it into
  source files or chapters before submitting those files.
- A conversion failure, including a PDF with no extractable text, returns a
  clear error identifying the source; the agent is not asked to do conversion.
- The original file is never modified. Generated Markdown is a managed RAG
  artifact traceable to that original source.

## Batch short-term memory MCP operation

Add `hieronymus_short_term_add_batch` for writing large reading results without
one MCP round-trip per conclusion.

- It accepts a session ID and an ordered list of the same item fields accepted
  by `hieronymus_short_term_add` except `session_id`.
- It validates every item before mutation, then writes the entire batch in one
  SQLite transaction.
- It returns ordered memory IDs and an item count.
- An invalid item rejects the whole batch and creates no memories.
- The server applies an explicit safe maximum batch size and reports it in its
  error. The initial limit must permit at least 500 short memories for a book
  reading workflow in a small number of requests.
- The Read skill uses batches and tells agents to continue until every important
  detail has an appropriate compact block.

## Soft concepts, terminology, and strict rules

Concepts are intentionally vague semantic entities. A concept may exist without
source form, rendering, aliases, or any terminology proposal.

Concept facets and terminology candidates can preserve source forms, preferred
renderings, and alternate renderings as searchable, advisory information. They
do not create strict validation requirements. Approval must no longer reject a
proposal solely because an approved variant differs from a canonical rendering.

Only rule crystals create strict terminology validation. A rule crystal is
created only when evidence establishes an explicit, stable rule, prohibition, or
user correction. Its canonical rendering and forbidden variants remain strict.

## Coverage-driven multi-pass dreaming

A dream run selects one bounded set of completed short-term memories and executes
separate provider passes over that same set. Each pass returns structured records
with the short-memory IDs that support them. A record without source evidence is
rejected.

The passes are ordered as follows:

1. **Concepts:** create or update concepts and facets.
2. **Terminology candidates:** create soft proposals with source forms,
   renderings, aliases, and rationale.
3. **Rule crystals:** create only explicit strict rules and prohibitions.
4. **Knowledge crystals:** create independent factual, narrative, stylistic,
   character, world, and analytical memory crystals.
5. **Relations:** create links among new and existing long-term records, with
   short-memory evidence.
6. **Reinforcement:** return evidence-backed existing long-term record IDs.
   Valid referenced long-term records are automatically reinforced and recorded
   in audit data.
7. **Coverage audit:** report every selected short-memory ID as used by one or
   more passes or as consciously omitted with a reason. Silent loss is invalid.

Passes are independently auditable and are not constrained to a small arbitrary
number of concepts, proposals, or crystals. Each provider prompt instructs the
model to maximize supported, non-duplicative extraction for its specific record
type. Bounded input and mutation limits remain to protect local operation.

## Configuration, UI, and observability

Dream configuration exposes individual workflow profiles/prompts for each pass,
with sensible defaults. Existing configurations migrate to compatible defaults.
The web admin shows per-pass input, output, accepted, rejected, reinforced,
covered, and consciously omitted counts, and lets the user inspect source-memory
and long-term evidence for a generated record.

Run audit records preserve prompts, selected short-memory IDs, pass outputs,
coverage results, long-term references, and automatic reinforcement decisions.

## Verification

Tests cover batch atomicity and a 500-item batch; each conversion policy and
error; independent Dream pass invocation and evidence validation; soft proposal
approval with divergent variants; strict rule-crystal validation; automatic
reinforcement; and coverage audit completeness. Integration coverage verifies a
book-scale batch is processed through all passes without silent short-memory
loss.
