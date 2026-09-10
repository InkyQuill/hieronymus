# Hieronymus Usage

For the long-term memory workflow, see [Memory Dreaming](memory-dreaming.md).

## Installation and Updates

The Rust candidate is a Linux x86_64 archive with one native executable, four
command names, an embedded console, and pinned semantic model/runtime assets.
It needs no Python, Node or Bun at runtime. It is not yet cleared for product
cutover: [the rehearsal](rust-cutover-rehearsal.md) records actual results and
open authority, native agent-host and service-manager gates.

Install a locally built, verified release into disposable roots:

```bash
scripts/install.sh --release-dir /path/to/release-dist \
  --app-dir /tmp/hiero-rehearsal/app --data-root /tmp/hiero-rehearsal/data \
  --unit-dir /tmp/hiero-rehearsal/units --no-activate
/tmp/hiero-rehearsal/app/bin/hiero version --json
```

The installer places each release under `app/versions/<version>` and switches
all stable `app/bin` aliases together. `--no-activate` stages the application
without contacting the user service manager. Launch the foreground daemon
for a disposable rehearsal:

```bash
/tmp/hiero-rehearsal/app/bin/hiero daemon --data-root /tmp/hiero-rehearsal/data --port 0
```

In another terminal, `hiero status --json --data-root <root>` reports the
authenticated daemon's semantic state. Only `ready` satisfies required
semantic readiness; acquiring, rebuilding and failed do not. `hiero stop
--data-root <root>` shuts down the owner. See [Distribution](distribution.md)
for verified source configuration, update options and recovery boundaries.
No public release URL is inferred from these local tests.

`hiero admin --data-root <root>` and `hiero config --data-root <root>` open
the embedded browser console through a single-use launch grant. Refreshes
reuse the browser cookie; restarting the daemon invalidates that session and
requires another launch. Explicit foreign Origins are refused. Generated
plugins use the stable `hieronymus-mcp` command, but current Claude/Codex
initialize requests are incompatible with mandatory MCP 2026-07-28; see
[actual host evidence](agent-host-acceptance.md).

## Data Root

By default, Hieronymus stores one global database at
`~/.config/hieronymus/hieronymus.sqlite`. Set `HIERONYMUS_DATA_ROOT` to use a
different data root:

```bash
export HIERONYMUS_DATA_ROOT=/home/inky/Yandex.Disk/Translation/.translation-memory
```

## Uninstall

`hiero uninstall --yes` removes the owned application, service unit and generated
integration entries; it preserves databases and configuration by default. Add
`--delete-data` only to remove the explicitly configured data root, including its
models, backups and audit data. Check `--data-root` or `HIERONYMUS_DATA_ROOT` before
using that option. A translation workspace should remain outside the application
data root; owned-path cleanup does not remove unrelated book directories.

## Configuration

Open the local configuration interface:

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

Example dreaming workflow assignments (the seven pass names are required; old
workflow names are rejected in this alpha schema):

```toml
# ~/.config/hieronymus/dream.conf
[dreaming]
enabled = true
schedule_interval_minutes = 30
min_pending_short_term_memories = 20
max_pending_short_term_memories = 200
max_short_term_memories_per_run = 500
max_long_term_records_affected_per_run = 1000
max_relation_records_per_pass = 1000
not_enough_memories_cycle_threshold = 5
max_changed_crystals_per_cycle = 200
max_related_concepts_per_cycle = 80
max_related_crystals_per_concept = 20
max_total_affected_crystals = 500
general_prompt = "Use English as the primary searchable memory language."

[workflows.knowledge_crystals]
provider = "openai"
model = "gpt-4.1-mini"
enabled = true
max_records_per_pass = 500
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

Manual `hiero dream` runs the configured seven Dream passes and drains all
pending short-term memories, including the final small batch that
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

Text, `.md`, and `.markdown` sources are ingested unchanged. Hieronymus converts
HTML, DOCX, and PDF into managed Markdown before indexing; EPUB is intentionally
rejected so an agent can split it deliberately.

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

## Project-Local Agent Skills

To install the bundled Hieronymus workflow skills into the current project for both supported
workspace conventions, run:

```bash
hiero skills install --target agents --target claude
```

This installs into `.agents/skills` and `.claude/skills`. It is distinct from the global
`hiero install <agent>` integration: project-local skills do not register MCP or modify host
configuration. Use `--dry-run` with either `hiero skills install` or `hiero skills uninstall` to
preview affected paths. Installation overwrites owned Hieronymus skill files; uninstallation removes
only the owned `hieronymus-*` skill directories and leaves unrelated project skills in place.

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

## Headless CLI

Every headless command goes through the local daemon (ADR 0009): the CLI
process never opens the database directly. Start the daemon with
`hiero daemon` (or `hiero service start`) first; when the daemon is not
running, commands report that honestly with the exact remediation instead of
writing around it. The one exception is `hiero export`, which is read-only.

### Call any advertised MCP tool

`hiero tool-call <tool> --args '<json>'` posts one stateless `tools/call` to
the daemon's authenticated `/mcp` route, so scripts can drive every tool an
MCP host can. `--start-daemon` opts into spawning the daemon; `--json` prints
the full MCP envelope.

Start or complete a session:

```bash
hiero tool-call hieronymus_series_create --args '{"slug":"oso","title":"Only Sense Online","source_language":"ja","target_language":"en"}'
hiero tool-call hieronymus_session_start --args '{"series_slug":"oso","volume":"01","chapter":"002"}'
hiero tool-call hieronymus_session_complete --args '{"session_id":1}'
```

Recall:

```bash
hiero tool-call hieronymus_recall --args '{"session_id":2,"series_slug":"oso","query":"cultural terms"}'
```

Dream over pending completed-session memories:

```bash
hiero tool-call hieronymus_dream --args '{}'
```

Dreaming through the tool runs the deterministic provider behind the
fail-closed workflow gate (the same path as the console's manual dreaming
action). Configured LLM provider lanes — the scheduler, draining, and
per-workflow providers — arrive with the dreaming plan; passing a named
provider is rejected instead of silently substituted.

RAG import and search:

```bash
hiero tool-call hieronymus_rag_import --args '{"series_slug":"oso","path":"/path/chapter-005.txt","source_ref":"book:5/chapter:5"}'
hiero tool-call hieronymus_rag_search --args '{"series_slug":"oso","query":"Cooking Talent"}'
```

Termbase validation (candidate rules stay advisory until an explicit
approval):

```bash
hiero tool-call hieronymus_termbase_propose --args '{"series_slug":"oso","category":"person_name","source_text":"ユン","canonical_translation":"Юн"}'
hiero tool-call hieronymus_termbase_approve --args '{"series_slug":"oso","term_id":1}'
hiero tool-call hieronymus_termbase_validate --args '{"series_slug":"oso","raw_text":"ユン stands up.","translated_text":"Юна встаёт."}'
```

### Export memory content as JSON

```bash
hiero export --output /path/hieronymus-memory.json [--json]
```

Export serializes the documented content tables (series, sessions,
short-term memories, crystals, concepts, facets, terminology rules, RAG
sources and chunks, dream runs) to one deterministic JSON document at the
explicit destination. It is a read operation: it opens the database
read-only and never copies a live SQLite file, so it is safe next to a
running daemon.

### Generate the agent plugin bundle

```bash
hiero plugins generate [--dry-run] [--json] [--data-root <path>]
```

Writes the installation-owned bundle under the config root's
`agent-plugins/` directory: the eight workflow skills, the MCP registration,
Codex hooks, local Claude/Codex marketplace catalogs, a passive Pi MCP/skills
package, and one manifest per other supported host (`codex`, `claude`,
`gemini`, `opencode`, `openclaw`). The MCP registration uses the stable
`hieronymus-mcp` entry point, which discovers the local daemon through the
data root's discovery record — generated configuration never contains a
fixed port or a bearer token. The command only writes Hieronymus-owned
files; it never rewrites your host configuration. Use `hiero uninstall` to
remove the bundle.

### Record recall feedback through the daemon

```bash
hiero recall-feedback --recall-id <id> --idempotency-key <key> [--useful <ids>] [--miss <ids>]
```

The CLI posts to the daemon's `POST /recall/feedback` route, so CLI, REST,
and MCP clients share one at-most-once feedback ledger and audit trail. It
requires the local daemon to be running and rejects `--start-daemon`
explicitly.

## Service Commands

```bash
hiero daemon
hiero service install
hiero service start
hiero service status --json
hiero service stop
hiero doctor
hiero semantic status
hiero tool-call hieronymus_series_list --args '{}'
hiero export --output ./memory.json
hiero plugins generate
hiero recall-feedback --recall-id <id> --idempotency-key <key>
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
