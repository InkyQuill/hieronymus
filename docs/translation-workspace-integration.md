# Translation Workspace Integration

Before translating a chapter, the orchestrator calls `hieronymus_termbase_contract` with the series slug, volume, and raw chapter text.

After translation, the orchestrator calls `hieronymus_termbase_validate` with the same raw chapter text and the translated chapter. Any high severity finding goes back to the Translator before Accuracy Review.

Fuzzy memories are advisory. Approved termbase entries are mandatory.
