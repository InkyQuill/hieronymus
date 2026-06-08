# Hieronymus

Hieronymus is a local-first translation memory MCP for long-form book translation.

It keeps strict terminology stable per series while also giving translator agents a searchable fuzzy memory for decisions, plot facts, voice notes, unresolved questions, and future memory-crystal consolidation.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/install.sh | sh
```

The installer creates a managed checkout at
`~/.local/share/hieronymus/app` and installs console commands through
`uv tool install`.

Update managed installs in place with:

```bash
hiero update
```

## Configuration

Open the local configuration TUI with:

```bash
hiero config
```

Textual remains the default TUI. The Ink/React preview is available with
`HIERONYMUS_TUI=ink hiero config` and `HIERONYMUS_TUI=ink hiero admin`; see the
[usage guide](docs/usage.md#ink-preview) for runtime requirements and keys.

For scripts and health checks, use machine-readable status:

```bash
hiero config --json
```

Non-secret settings are stored in `~/.config/hieronymus/settings.toml`.
API key values are not stored. Provider entries store environment variable names,
and runtime provider calls read the secret value from the environment.

Supported dream providers:

- `deterministic`: offline local fallback.
- `openai`: OpenAI and OpenAI-compatible endpoints using `OPENAI_API_KEY`.
- `gemini`: Gemini API using `GEMINI_API_KEY`.
- `anthropic`: Anthropic Messages API using `ANTHROPIC_API_KEY`.

Dreaming automation is controlled by `autostart_enabled`,
`min_interval_minutes`, `new_short_term_memory_threshold`, and
`max_cycles_per_autostart`.

## Frontend Development

The Ink/React frontend lives under `frontend/`. Use pnpm from the repository
root:

```bash
pnpm --dir frontend install
pnpm --dir frontend test
pnpm --dir frontend build
```

Command summary:

- `hiero config` edits dream provider and autostart settings in a local TUI.
- `hiero config --json` prints secret-safe provider and dreaming status for
  automation.
- `hiero dream --wait` waits for an active dream cycle instead of failing fast.

Uninstall the app with:

```bash
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/uninstall.sh | sh
```

The non-interactive uninstall one-liner removes the app and keeps settings/data
by default.

To choose data handling explicitly:

```bash
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/uninstall.sh | sh -s -- --keep-data
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/uninstall.sh | sh -s -- --purge-data
```

For an interactive prompt, run the managed checkout script from a terminal:

```bash
~/.local/share/hieronymus/app/uninstall.sh
```

The prompt uses ~/.config/hieronymus unless HIERONYMUS_DATA_ROOT is set.

--purge-data removes the configured data root. If HIERONYMUS_DATA_ROOT is
set, check it before purging.

The uninstall script only removes Hieronymus-owned install and config/data paths.
It does not remove translation workspace directories.

Repository: <https://github.com/InkyQuill/hieronymus>

## Status

MVP implementation exists for local series setup, strict termbase validation, memory import/search, and MCP/CLI workflows.

## Documents

- [Usage guide](docs/usage.md)
- [Management TUI usage](docs/usage.md#management-tui)
- [Agent workflows](docs/agent-workflows.md)
- [Service toolkit](docs/service-toolkit.md)
