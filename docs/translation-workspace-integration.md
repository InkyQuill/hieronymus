# Translation Workspace Integration

Before translating a chapter, the orchestrator calls
`hieronymus_termbase_contract(series_slug, raw_text)`. The tool name is kept for
compatibility; the contract is generated from active rule crystals.

After translation, the orchestrator calls
`hieronymus_termbase_validate(series_slug, raw_text, translated_text)` with the
same raw chapter text and the translated chapter. Any high severity finding goes
back to the Translator before Accuracy Review.

Volume, chapter, and other story position markers are freeform workflow context.
They boost recall relevance without filtering unrelated memories out of search.

Fuzzy memories are advisory. Active rule crystals are mandatory until
superseded or combined by later rule crystals.

## Supporting MCP Tools

- `hieronymus_termbase_propose(series_slug, category, source_text, canonical_translation, tags=None, notes="")`
- `hieronymus_termbase_approve(series_slug, term_id)`
- `hieronymus_memory_search(series_slug, query, limit=5)`
- `hieronymus_memory_add(series_slug, kind, text, source_ref="", importance=3)`

These older tool names are compatibility wrappers. `hieronymus_memory_add`
creates short-term memory that stays pending until dreaming processes it.
Corrections and approved proposal compatibility flows become high-confidence rule
crystals instead of bypassing the learning workflow.
