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

Textual is the default TUI. In source checkouts, the Ink/React TUI is available
as a preview after the frontend has been built and Node.js >=22 is installed:

```bash
HIERONYMUS_TUI=ink hiero config
```

For machine-readable status, use:

```bash
hiero config --json
```

The config interface exposes the primary dreaming config from
`~/.config/hieronymus/dream.conf`: schedule/minimum/urgent thresholds, named
provider profiles, workflow-to-provider/model assignments, cached model
suggestions from `llmcache.tmp`, and dreaming prompts. Provider profiles store
their endpoint, model hints, and API key directly in `dream.conf`; the file is
plain text local configuration.

Edits stay in memory until saved. Reload discards unsaved edits and reads
`dream.conf` again. Provider checks use the edited profile, refresh model
suggestions where the provider supports model listing, and update
`llmcache.tmp`.

Supported dream provider profile types:

- `openai`: OpenAI and OpenAI-compatible endpoints.
- `gemini`: Gemini API.
- `anthropic`: Anthropic Messages API.
- `ollama`: local Ollama chat/model endpoints.

In the Ink config TUI, the remote provider selector offers `openai`, `gemini`,
`anthropic`, and local-compatible profiles as the implementation supports them.

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

Textual is the default admin TUI. In source checkouts, the Ink/React admin
preview is available after the frontend has been built:

```bash
HIERONYMUS_TUI=ink hiero admin
```

The TUI is a local-first management surface for reviewing and controlling
Hieronymus memory data. It shows global status and statistics, then lets an
admin switch between crystals, lessons, concepts, legacy compatibility proposal
records, dream runs, and audit events. Each view supports keyboard navigation
through entries, filter dialogs, a detail pane, and command actions that match
the selected entry type.

Useful controls:

- `1`-`8`: switch views
- `j` / `k`: move through entries
- `f` or `/`: filter the current view
- `e`: edit the selected crystal or lesson
- `a`: approve the selected legacy compatibility proposal record
- `x`: reject the selected legacy compatibility proposal record
- `+` / `-`: reinforce or decay the selected crystal or lesson
- `d`: deprecate the selected crystal or lesson
- `delete`: delete after confirmation
- `p`: inspect provenance for the selected entry
- `ctrl+p`: open the command palette

The command palette exposes the broader admin action surface where the selected
view supports it: add, edit, delete, merge, split, supersede, reinforce, decay,
promote a local lesson to a global candidate, activate a global lesson, inspect
provenance, inspect recall reasons, run manual dreaming, review dream outputs,
and review concept, facet, rule-crystal, or legacy compatibility proposal
records.

For scripts and health checks, use:

```bash
hiero admin --json
```

This prints management counts and available TUI views without opening the
interactive app.

## Ink Preview

The Ink TUI is feature-flagged with `HIERONYMUS_TUI=ink`; Textual remains the
default unless that environment variable is set.

Ink can become the default only after all default-switch criteria are verified
for the same release candidate:

- Installed packages include a bundled, self-contained
  `hieronymus/frontend/dist/main.js` frontend artifact before installed
  packages default to Ink.
- Python verification passes with `uv run pytest`, `uv run ruff check .`, and
  `uv run ruff format --check .`.
- Frontend verification passes with `pnpm --dir frontend install`,
  `pnpm --dir frontend test`, and `pnpm --dir frontend build`.
- Non-interactive config and admin smoke tests pass with `hiero config --json`
  and `hiero admin --json` against a temporary data root.
- Ink config and admin smoke tests pass on supported platforms, or documented
  manual checks cover the interactive `HIERONYMUS_TUI=ink hiero config` and
  `HIERONYMUS_TUI=ink hiero admin` flows.
- Textual fallback remains verified with `HIERONYMUS_TUI=textual` for config
  and admin during the switch window.
- User-facing docs are updated and `hiero doctor` verifies the required Ink
  runtime.

Ink config MVP keys:

- `1` / `2` / `3`: select `openai`, `gemini`, or `anthropic`
- `s`: save
- `r`: reload
- `c`: check the selected provider
- `q`: quit

Ink admin MVP keys:

- `1`-`8`: switch views
- `f`: select filter command
- `e`: select edit command
- `+`: reinforce the selected crystal or lesson
- `-`: decay the selected crystal or lesson
- `d`: delete the selected crystal or lesson
- `ctrl+p`: toggle command palette
- `q`: quit

Frontend development and source-checkout preview builds require Node.js >=22 and
pnpm:

```bash
pnpm --dir frontend install
pnpm --dir frontend build
```

After that build, `HIERONYMUS_TUI=ink` uses the CLI fallback to
`frontend/dist/main.js` from the current working directory. Installed packages
should not be treated as shipping a self-contained Ink frontend until release
tooling produces a bundled `hieronymus/frontend/dist/main.js` artifact.
