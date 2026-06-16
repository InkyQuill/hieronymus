# Hieronymus

![CodeRabbit Pull Request Reviews](https://img.shields.io/coderabbit/prs/github/InkyQuill/hieronymus?utm_source=oss&utm_medium=github&utm_campaign=InkyQuill%2Fiview&labelColor=171717&color=FF570A&link=https%3A%2F%2Fcoderabbit.ai&label=CodeRabbit+Reviews)

Hieronymus is alpha local-first translation memory software for long-form book
translation. It can be used today, but it is still changing quickly and should
be used at your own risk.

It keeps strict terminology stable per series while also giving translator agents a searchable fuzzy memory for decisions, plot facts, voice notes, unresolved questions, and future memory-crystal consolidation.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/InkyQuill/hieronymus/main/install.sh | sh
```

The installer creates a managed checkout at
`~/.local/share/hieronymus/app` and installs console commands through
`uv tool install`. In an interactive terminal it asks whether to install the
stable channel or the dev channel. Stable installs the latest tagged alpha
release; dev installs the latest `main` commit. For non-interactive installs,
the default is stable. Set `HIERONYMUS_INSTALL_CHANNEL=dev` to install from
`main`.

Update managed installs in place with:

```bash
hiero update
```

## Configuration

Open the local configuration TUI with:

```bash
hiero config
```

The TUI uses a TypeScript React/OpenTUI terminal UI. See the
[usage guide](docs/usage.md) for details, runtime requirements, and keys.

For scripts and health checks, use machine-readable status:

```bash
hiero config --json
```

Dreaming configuration is stored in plaintext local config under the configured
Hieronymus data root. API key values may be stored locally and are redacted from
doctor output, JSON bridge responses, logs, provider checks, and audit records.

Supported dream providers:

- `deterministic`: offline local fallback.
- `openai`: OpenAI and OpenAI-compatible endpoints using `OPENAI_API_KEY`.
- `gemini`: Gemini API using `GEMINI_API_KEY`.
- `anthropic`: Anthropic Messages API using `ANTHROPIC_API_KEY`.

Dreaming automation is controlled by `autostart_enabled`,
`min_interval_minutes`, `new_short_term_memory_threshold`, and
`max_cycles_per_autostart`.

## Frontend Development

The React/OpenTUI frontend lives under `frontend/`. Use Bun >=1.3 from the repository
root:

```bash
bun install --cwd frontend --frozen-lockfile
bun run --cwd frontend build
bun run --cwd frontend test
```

After `bun run --cwd frontend build`, source checkouts can launch the TUI
through the CLI fallback to `frontend/dist/main.js`. Installed packages
bundle the `hieronymus/frontend/dist/main.js` artifact automatically.

Command summary:

- `hiero config` edits local `dream.conf`, `provider.conf`, `ingest.conf`,
  and `release.conf` settings in a local TUI.
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

Alpha implementation exists for local series setup, rule-crystal validation,
memory import/search, MCP/CLI workflows, dreaming, and the React/OpenTUI
management app. No 1.x release is approved yet.

## Documents

- [Usage guide](docs/usage.md)
- [Management TUI usage](docs/usage.md#management-tui)
- [Agent workflows](docs/agent-workflows.md)
- [Service toolkit](docs/service-toolkit.md)
- [Roadmap](docs/roadmap.md)
