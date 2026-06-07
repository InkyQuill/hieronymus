# Hieronymus Config TUI and LLM Dreaming

## Goal

Hieronymus needs a real configuration control surface before normal users can run it confidently. The
current `hiero config` command still reports paths and a placeholder TUI state, while dreaming is
hard-coded to the deterministic provider. This pass replaces that with a working config TUI, persisted
settings, provider health checks, real OpenAI and Gemini dreaming providers, and cycle-based dreaming
automation controls. There must be no remaining `not-available-in-this-pass` or equivalent placeholder
claims for this area.

## Scope

The first supported LLM provider priority is:

1. OpenAI and OpenAI-compatible endpoints.
2. Gemini.
3. Anthropic.

OpenAI and Gemini must be usable for real dream cycles in this pass. Anthropic is the next provider in
priority order and should be implemented after OpenAI and Gemini if it fits the same adapter pattern.
If Anthropic is not implemented, it must not appear as a selectable provider, documented setup path, or
placeholder status. The deterministic provider remains available as a local fallback, test provider,
and offline mode.

The configuration TUI must control:

- active dreaming provider
- provider enablement
- provider model
- provider base URL where relevant
- provider API key environment variable name
- one-off provider connectivity checks
- dreaming automation enabled/disabled
- minimum minutes between automatic dream cycles
- new short-term memory threshold for immediate automatic dreaming
- maximum dream cycles per automatic start
- service/path diagnostics needed to understand where data and settings live

## Configuration Model

Add a persisted `settings.toml` under the configured Hieronymus root, defaulting to
`~/.config/hieronymus/settings.toml`. This file stores non-secret operational settings only. API key
values are not saved. Provider settings store environment variable names such as `OPENAI_API_KEY`,
`GEMINI_API_KEY`, or `ANTHROPIC_API_KEY`.

The settings layer should have a small typed model with defaults and validation. Loading settings must
be tolerant of a missing file and strict about malformed values. Saving settings must be atomic and
must preserve the rule that config and runtime files stay under the Hieronymus data/config root.

Suggested default settings:

```toml
[dreaming]
active_provider = "deterministic"
autostart_enabled = false
min_interval_minutes = 30
new_short_term_memory_threshold = 25
max_cycles_per_autostart = 1

[providers.deterministic]
enabled = true

[providers.openai]
enabled = false
model = "gpt-4.1-mini"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[providers.gemini]
enabled = false
model = "gemini-2.5-flash"
api_key_env = "GEMINI_API_KEY"
```

Model defaults may be adjusted during implementation if current provider recommendations change, but
tests and docs must make the chosen defaults explicit.

## Secret Handling

Hieronymus should not write API key values into `settings.toml`. The TUI may offer a temporary secret
input for a provider check, but that value is used only for the current check and discarded. Normal
runtime provider resolution reads from the configured environment variable. Error messages must name
the missing environment variable, not echo a secret.

## Provider Architecture

Replace hard-coded provider selection in the CLI, admin store, and MCP path with a provider registry
and factory. The registry exposes:

- provider metadata for TUI rendering
- config validation
- connectivity check support
- creation of a `DreamProvider`
- structured status for CLI, TUI, service status, and doctor

All providers must return the existing `DreamOutput` shape. LLM providers must use structured prompts
and strict response validation before any output is applied. Existing safeguards remain in force:
invalid provider output records a failed dream run and does not mark sessions as dreamed or insert
partial outputs.

OpenAI-compatible support should use the configured `base_url`, model, timeout, and API key env var.
Gemini support should use the Gemini API directly with the configured model and API key env var.
Anthropic support should follow the same boundary if implemented in this pass.

Provider health checks should be lightweight. They verify that required config is present, required
secret env vars or temporary keys are available, and the remote provider can respond to a small
non-dreaming check. Check results should include status, provider name, model, latency if available,
and a concise error string.

## Dreaming Automation

Automatic dreaming is cycle-based, not wall-clock decay-based. It may start a dream cycle only when
there are pending short-term memories in completed sessions.

Two triggers are supported:

- interval trigger: once every `N` minutes if pending short-term memories exist
- volume trigger: immediately when the count of new short-term memories since the last dream reaches
  `M`

The "new short-term memories" threshold counts short-term memory rows, not long-term remembered
crystals or strict terminology rows. The implementation should define the counter against sessions
that are completed and not yet dreamed, because those are the inputs the dream service can safely
process.

`max_cycles_per_autostart` limits how many cycles can run from one automatic start. The default should
be conservative, and failures must stop the current automatic run rather than looping.

## Config TUI

`hiero config` opens a Textual TUI. `hiero config --json` remains a non-interactive probe and should
report real settings/status, not a placeholder.

The TUI should be keyboard-first and dense, matching the admin TUI style. It should have clear sections
or tabs:

- Providers: list configured providers, active provider, enabled state, model, key source, and health.
- Dreaming: edit automation triggers and active provider.
- Service: show daemon status, version, provider status, and housekeeping/dream automation state.
- Paths: show config root, settings path, database path, backups root, and plugin assets root.
- Diagnostics: run provider checks and config validation checks.

Expected operations:

- switch active provider
- enable or disable a provider
- edit provider model/base URL/API key env var
- run a provider connectivity check
- edit dreaming automation settings
- save settings atomically
- reload settings from disk
- show validation errors without writing invalid config

The TUI must not become a separate source of truth. It calls the settings/provider facade and renders
the result.

## CLI, Service, Doctor, and MCP Behavior

`hiero dream` should default to the configured active provider. `--provider` can override it for a
single run. Unsupported or disabled providers should fail with a clear Click error. `hiero dream
--json` should return stable automation output with cycle id, provider, status, input count, created
crystal count, proposal count, and error text when present.

`hiero status` and service `/status` should report provider status and dreaming automation state. The
current empty `providers: []` response should be replaced with useful structured provider data.

`hiero doctor` should validate:

- settings file parseability
- active provider exists
- active provider is enabled
- required provider environment variable is present for non-deterministic providers
- provider check failures as warnings or errors depending on whether that provider is active

`hieronymus_dream` in MCP should accept configured providers rather than only `deterministic`, while
preserving the same strict validation and rollback behavior as CLI dreaming.

## Documentation

Update README and docs so they describe the real config TUI and provider behavior. Remove statements
that external LLM providers, config TUI, or provider checks are deferred. Documentation should include:

- `hiero config`
- `hiero config --json`
- OpenAI setup with env var
- Gemini setup with env var
- deterministic fallback
- dreaming automation settings
- provider check behavior
- security note that API key values are not stored

## Error Handling

Failures should be explicit and repairable:

- malformed `settings.toml`: report parse/validation error and do not silently reset
- missing API key env var: name the variable and provider
- disabled active provider: block dreaming until fixed
- remote provider failure: record failed check or failed dream run with concise error
- invalid LLM JSON/schema: fail the dream run and preserve pending sessions
- automatic dreaming failure: stop that automatic run and surface status for doctor/TUI

## Testing

Tests should cover:

- settings defaults, parse, validation, and atomic writes
- no secret values written to settings
- config CLI JSON reports real settings and paths
- config TUI launches and can edit/save core settings in a Textual pilot test
- provider registry resolves deterministic, OpenAI, and Gemini
- provider checks handle missing env vars, temporary keys, success, and remote failure
- OpenAI/Gemini providers parse valid structured responses into `DreamOutput`
- invalid LLM responses fail without partial database mutation
- `hiero dream` uses the active provider by default and supports `--provider`
- service status and doctor include provider/dreaming settings
- docs no longer contain "not-available-in-this-pass" or equivalent deferred claims for config TUI or
  external LLM providers

Network tests must be mocked. The implementation should not require real API keys in CI.

## Non-Goals

This pass does not need to build a web UI, OS keychain integration, scheduled OS services, or a
background process manager beyond the existing service daemon path. It should not promote LLM output
to strict terminology without the existing proposal/review path.
