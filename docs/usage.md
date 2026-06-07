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

For machine-readable status, use:

```bash
hiero config --json
```

Non-secret settings are stored in `~/.config/hieronymus/settings.toml` by
default, or in `settings.toml` under the configured `HIERONYMUS_DATA_ROOT`.
API key values are not stored. Provider entries store the environment variable
name for each key, and dream runs read the secret value from the runtime
environment.

Supported dream providers:

- `deterministic`: offline local fallback.
- `openai`: OpenAI and OpenAI-compatible endpoints using `OPENAI_API_KEY`.
- `gemini`: Gemini API using `GEMINI_API_KEY`.
- `anthropic`: Anthropic Messages API using `ANTHROPIC_API_KEY`.

Example OpenAI-backed dreaming run:

```bash
export OPENAI_API_KEY=...
hiero config
hiero dream --provider openai --json
```

Dreaming automation uses `autostart_enabled`, `min_interval_minutes`,
`new_short_term_memory_threshold`, and `max_cycles_per_autostart`.

## Initialize a Series

```bash
hieronymus init-series only-sense-online --title "Only Sense Online" --source-language ja --target-language en
hieronymus init-series death-march --title "Death March to the Parallel World Rhapsody" --source-language ja --target-language en
```

## Propose a Term

```bash
hieronymus propose-term only-sense-online --category person_name --source "ユン" --translation "Yun" --tag name
```

## Validate a Chapter

```bash
cd /home/inky/Yandex.Disk/Translation
hieronymus validate only-sense-online --raw-file only-sense-online/vol01/raw/chapter-002.xhtml --translated-file only-sense-online/vol01/translated/chapter-002.md
```

## Remember Translation Context

```bash
hieronymus remember only-sense-online --kind translation_rationale --text "Use Yun for ユン." --source-ref only-sense-online/vol01/chapter-002
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

The TUI is a local-first management surface for reviewing and controlling
Hieronymus memory data. It shows global status and statistics, then lets an
admin switch between crystals, lessons, concepts, proposals, dream runs, and
audit events. Each view supports keyboard navigation through entries, filter
dialogs, a detail pane, and command actions that match the selected entry type.

Useful controls:

- `1`-`8`: switch views
- `j` / `k`: move through entries
- `f` or `/`: filter the current view
- `e`: edit the selected crystal or lesson
- `a`: approve the selected proposal
- `x`: reject the selected proposal
- `+` / `-`: reinforce or decay the selected crystal or lesson
- `d`: deprecate the selected crystal or lesson
- `delete`: delete after confirmation
- `p`: inspect provenance for the selected entry
- `ctrl+p`: open the command palette

The command palette exposes the broader admin action surface where the selected
view supports it: add, edit, delete, merge, split, supersede, reinforce, decay,
promote a local lesson to a global candidate, activate a global lesson, inspect
provenance, inspect recall reasons, run manual dreaming, review dream outputs,
and approve or reject strict-concept proposals.

For scripts and health checks, use:

```bash
hiero admin --json
```

This prints management counts and available TUI views without opening the
interactive app.
