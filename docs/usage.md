# Hieronymus Usage

For the long-term memory workflow, see [Memory Dreaming](memory-dreaming.md).

## Installation and Updates

Install Hieronymus with:

```bash
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/install.sh | sh
```

The installer keeps the managed application checkout at
`~/.local/share/hieronymus/app` and installs the `hieronymus`, `hiero`, and
`hieronymus-mcp` console commands through `uv tool install`. If `hiero` is not
available after installation, add `~/.local/bin` to `PATH`.

In an interactive terminal, the installer asks whether to install the stable or
dev channel. Stable installs the latest tagged alpha release and dev installs
the latest `main` commit. Non-interactive installs default to stable. To choose
the channel explicitly:

```bash
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/install.sh | HIERONYMUS_INSTALL_CHANNEL=stable sh
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/install.sh | HIERONYMUS_INSTALL_CHANNEL=dev sh
```

The installer writes the selected update channel to `release.conf`, so later
`hiero update` calls follow the same stable or dev channel.

Update an installed checkout:

```bash
hiero update
```

Check for updates without applying them:

```bash
hiero update --check
```

Uninstall the app:

```bash
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/uninstall.sh | sh
```

The non-interactive uninstall one-liner removes the app and keeps settings/data
by default.

Choose data handling explicitly:

```bash
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/uninstall.sh | sh -s -- --keep-data
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/uninstall.sh | sh -s -- --purge-data
```

For an interactive prompt, run the managed checkout script from a terminal:

```bash
~/.local/share/hieronymus/app/uninstall.sh
```

The uninstall script only removes Hieronymus-owned install and config/data
paths. It does not remove translation workspace directories.

--purge-data removes the configured data root. If HIERONYMUS_DATA_ROOT is
set, check it before purging.

Unset or check `HIERONYMUS_DATA_ROOT` before using `--purge-data` if it points
at data you want to keep, such as a data root inside a translation workspace.

## Data Root

By default, Hieronymus stores one global database at
`~/.config/hieronymus/hieronymus.sqlite`. Set `HIERONYMUS_DATA_ROOT` to use a
different data root:

```bash
export HIERONYMUS_DATA_ROOT=/home/inky/Yandex.Disk/Translation/.translation-memory
```

## Configuration

Open the local configuration TUI:

```bash
hiero config
```

The command starts the loopback-only service when needed and opens the local Svelte
web console in the default browser. The bootstrap token is exchanged for an HttpOnly
local-session cookie before the app loads.

For machine-readable status, use:

```bash
hiero config --json
```

The config interface edits local plaintext config files: `provider.conf` for
provider endpoints, defaults, and API keys; `dream.conf` for workflow model
assignments, prompts, thresholds, and caps; `ingest.conf` for memory ingestion
limits; and `release.conf` for update channel selection. JSON output, logs,
provider checks, doctor output, and dream audit payloads redact configured
provider API keys.

Edits stay in memory until saved. Reload discards unsaved edits and reads the
local config files again. Provider checks use the edited profile, refresh model
suggestions where the provider supports model listing, and update
`llmcache.tmp`.

Supported dream provider profile types:

- `openai`: OpenAI and OpenAI-compatible endpoints.
- `gemini`: Gemini API.
- `anthropic`: Anthropic Messages API.
- `ollama`: local Ollama chat/model endpoints.

The Providers page supports any number of named profiles. Dreaming uses a discovered
model select when the profile exposes models and a model-ID field otherwise.

Example provider catalog:

```toml
# ~/.config/hieronymus/provider.conf
[defaults]
provider = "openai"
model = "gpt-4.1-mini"

[openai]
name = "OpenAI"
type = "openai"
url = "https://api.openai.com/v1"
key = "sk-local-plaintext-value"
timeout_seconds = 30.0
```

Example dreaming workflow assignments:

```toml
# ~/.config/hieronymus/dream.conf
[dreaming]
enabled = true
schedule_interval_minutes = 30
min_pending_short_term_memories = 20
max_pending_short_term_memories = 200
max_short_term_memories_per_cycle = 50
not_enough_memories_cycle_threshold = 5
max_changed_crystals_per_cycle = 200
max_related_concepts_per_cycle = 80
max_related_crystals_per_concept = 20
max_total_affected_crystals = 500
general_prompt = "Use English as the primary searchable memory language."

[workflows.crystallization]
provider = "openai"
model = "gpt-4.1-mini"
enabled = true
```

Model suggestions appear when the selected provider API supports listing models
and the configured profile can be reached. If model listing is unavailable, the
TUI shows cached/default hints and `hiero doctor` reports stale, unreachable, or
missing-model conditions.

Example OpenAI-backed dreaming run:

```bash
hiero config
hiero doctor
hiero dream --json
```

Manual `hiero dream` uses the configured crystallization workflow profile and
drains all pending short-term memories, including the final small batch that
scheduled dreaming would normally leave until the minimum threshold is met. Use
`--wait` to block until an active dream cycle finishes, and `--json` for
machine-readable output.

Scheduled dreaming respects the configured minimum pending-memory threshold
unless the urgent cap or backlog escape rule fires.

## Initialize a Series

```bash
hieronymus init-series only-sense-online --title "Only Sense Online" --source-language ja --target-language en
hieronymus init-series death-march --title "Death March to the Parallel World Rhapsody" --source-language ja --target-language en
```

The CLI options are compatibility hints for existing workflows. The current
memory model treats a series as language-neutral and stores active languages as
language tags on series, sessions, facets, short-term memories, and crystals.

## Store a Concept With Facets

Agent integrations should use primitive MCP/admin operations when they already
know a concept and its metadata. This example creates one concept with an English
canonical name, a Japanese source form, a Russian rendering, semantic tag
`talent`, and story scope `book:5/chapter:5`:

```text
hieronymus_series_create(
  slug="only-sense-online",
  title="Only Sense Online",
  language_tags=["ja", "en", "ru"],
)

hieronymus_concept_create(
  canonical_name="Cooking Talent",
  series_slug="only-sense-online",
  semantic_tags=["talent"],
  status="established",
)

hieronymus_concept_facet_add(
  concept_id=<concept_id>,
  value="Cooking Talent",
  language_tags=["en"],
  kind="name",
  is_canonical=true,
  story_scopes=["book:5/chapter:5"],
  semantic_tags=["talent"],
)

hieronymus_concept_facet_add(
  concept_id=<concept_id>,
  value="料理",
  language_tags=["ja"],
  kind="source_form",
  story_scopes=["book:5/chapter:5"],
  semantic_tags=["talent"],
)

hieronymus_concept_facet_add(
  concept_id=<concept_id>,
  value="Готовка",
  language_tags=["ru"],
  kind="rendering",
  story_scopes=["book:5/chapter:5"],
  semantic_tags=["talent"],
)
```

Use rule-crystal admin actions only to inspect, validate, archive, or otherwise
manage existing long-term rules. User corrections should enter as short-term
memory and be crystallized by dreaming.

## Import Project RAG Sources

RAG sources are explicit project text and glossary files. They are advisory
evidence for recall; active rule crystals remain mandatory.

Import a text or Markdown file:

```bash
hiero rag import only-sense-online ./chapter-005.txt --source-ref book:5/chapter:5/source.txt
```

Import a glossary:

```bash
hiero rag import only-sense-online ./glossary.csv --type glossary --source-ref glossary/main.csv
```

Search RAG evidence directly:

```bash
hiero rag search only-sense-online "Cooking Talent" --json
```

Ordinary recall includes both memory results and RAG evidence. RAG entries include
their source reference, chunk kind, location, score, and rank reason so agents can
cite where evidence came from.

## Compatibility Term Proposal

The term proposal command remains available for older translation workflows, but
new agent workflows should prefer concept, facet, short-term memory, and
rule-crystal primitives.

```bash
hieronymus propose-term only-sense-online --category person_name --source "ユン" --translation "Yun" --tag name
```

## Validate a Chapter

```bash
cd /home/inky/Yandex.Disk/Translation
hieronymus validate only-sense-online --raw-file only-sense-online/vol01/raw/chapter-002.xhtml --translated-file only-sense-online/vol01/translated/chapter-002.md
```

## Agent Memory Skills

Read, Learn, and Remember are agent skill workflows, not preferred MCP judgment
tools. The agent decides what is worth recording, how credible it is, and which
language tags, story scopes, semantic tags, concept links, or source references
apply. It then calls storage and retrieval primitives such as
`hieronymus_short_term_add`, `hieronymus_recall`, concept primitives, and facet
primitives.

There is no preferred `hieronymus_read` or `hieronymus_learn` interface. Those
judgment-heavy wrappers are no longer exposed as the current MCP workflow. See
[Read, Learn, And Remember Skills](skills/read-learn-remember.md) and
[Agent workflows](agent-workflows.md).

For a high-credibility correction, Remember should store a short memory like:

```text
hieronymus_short_term_add(
  session_id=2,
  source_role="user",
  kind="correction",
  text="User told me to render Cooking Talent as Готовка in Russian.",
  language_tags=["en", "ja", "ru"],
  story_scopes=["book:5/chapter:5"],
  semantic_tags=["talent"],
  source_credibility="user_rule",
  rule_intent="terminology",
)
```

For recall, the agent opens or reuses an active session and calls the primitive
retrieval tool directly:

```text
hieronymus_session_start(
  series_slug="only-sense-online",
  task_type="translation",
  volume="05",
  chapter="005",
)

hieronymus_recall(
  session_id=<session_id>,
  series_slug="only-sense-online",
  query="Cooking Talent Russian rendering",
  limit=10,
)
```

## Memory Dreaming Workflow

```bash
hieronymus init-series oso --title "Only Sense Online" --source-language ja --target-language en
hieronymus session-start oso --source-language ja --target-language en --task-type translation --volume 01 --chapter 002
hieronymus remember-short 1 --role user --kind correction --text "Define obscure Japanese cultural terms when the average English reader may not know them."
hieronymus session-complete 1
hieronymus dream --provider deterministic
hieronymus session-start oso --source-language ja --target-language en --task-type translation --volume 01 --chapter 002
hieronymus recall 2 --series oso --source-language ja --target-language en --task-type translation --volume 01 --chapter 002 --query "cultural terms"
```

The final recall command uses session `2` because recall must run inside a new
active session after session `1` has been completed and dreamed.
Corrections enter the workflow as short-term memories and become rule crystals
through dreaming.

## Service Commands

```bash
hiero
hiero status --json
hiero doctor
hiero admin
hiero admin --json
hiero install codex --dry-run
hiero stop
```

`hiero` is an alias for `hieronymus`; all subcommands work with either command.

## Management TUI

Open the local admin interface with:

```bash
hiero admin
```

The admin command opens the same local web console at its administration route.

The TUI is a local-first management surface for reviewing and controlling
Hieronymus memory data. It shows global status and statistics, then lets an
admin switch between crystals, lessons, concepts, legacy compatibility proposal
records, dream runs, and audit events. Each view supports keyboard navigation
through entries, filter dialogs, a detail pane, and command actions that match
the selected entry type.

The command palette is backed by Python command metadata. It shows only
commands relevant to the current view, marks commands that require a selected
row as unavailable when no row is selected, and executes actions through
AdminBridge RPC rather than parsing CLI output.

For scripts and health checks, use:

```bash
hiero admin --json
```

This prints management counts and available views without opening the
interactive app.

## Local Web Console

The configuration and administration pages are served by the loopback-only
Hieronymus service. `hiero config` and `hiero admin` open the relevant page in a
browser; the bootstrap URL is converted to an HttpOnly local-session cookie.

### Frontend Development

Frontend development and source-checkout builds require Bun >=1.3:

```bash
bun install --cwd frontend --frozen-lockfile
bun run --cwd frontend build
```

The wheel packages the resulting `frontend/dist` assets for the local service.
