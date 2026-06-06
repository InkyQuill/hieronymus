# Translation Workspace Integration

Before translating a chapter, the orchestrator calls `hieronymus_termbase_contract(series_slug, raw_text)`.

After translation, the orchestrator calls `hieronymus_termbase_validate(series_slug, raw_text, translated_text)` with the same raw chapter text and the translated chapter. Any high severity finding goes back to the Translator before Accuracy Review.

Volume remains external workflow context for selecting and tracking chapters; it is not an MVP MCP tool argument.

Fuzzy memories are advisory. Approved termbase entries are mandatory.

## Supporting MCP Tools

- `hieronymus_termbase_propose(series_slug, category, source_text, canonical_translation, tags=None, notes="")`
- `hieronymus_termbase_approve(series_slug, term_id)`
- `hieronymus_memory_search(series_slug, query, limit=5)`
- `hieronymus_memory_add(series_slug, kind, text, source_ref="", importance=3)`
