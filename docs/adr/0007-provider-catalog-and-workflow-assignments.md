# Split Provider Catalog From Workflow Assignments

## Status

Accepted.

## Context

The current dreaming configuration mixes two different responsibilities in
`dream.conf`:

- provider profiles, including endpoint URLs and plaintext API keys;
- workflow assignments, such as which provider and model crystallization uses.

This makes `dream.conf` grow provider templates that are not specific to
dreaming and forces `hiero config` into a single selected-provider model. That
model is too narrow: different LLM-consuming workflows need to use different
configured endpoints and models. For example, crystallization may use Deepseek
through an OpenAI-compatible API, reinforcement compaction may use local
Ollama, and future LLM tasks may use other configured services.

Provider profiles are local user configuration, not book project data. They
belong under the Hieronymus data root alongside other local configuration.

## Decision

Introduce `provider.conf` as the global local catalog of configured LLM
provider profiles. `provider.conf` lives in the Hieronymus data root and owns
endpoint access details:

```toml
[deepseek-api]
name = "Deepseek"
type = "openai"
url = "https://api.deepseek.com"
key = "..."

[local-ollama]
name = "Ollama"
type = "openai"
url = "http://127.0.0.1:6000/v1"

[google-api]
name = "Google"
type = "google"
url = "https://generativelanguage.googleapis.com"
key = "..."

[defaults]
provider = "deepseek-api"
model = "deepseek-v4-flash"
```

`dream.conf` no longer owns provider endpoint templates or API keys. It owns
dreaming settings and per-workflow LLM assignments. Every LLM-consuming
workflow assignment has the same shape: `provider`, `model`, and `enabled`.

```toml
[workflows.crystallization]
provider = "deepseek-api"
model = "deepseek-v4-flash"
enabled = true

[workflows.reinforcement_compaction]
provider = "local-ollama"
model = "gemma4-e3b"
enabled = true

[workflows.relation_discovery]
provider = "deepseek-api"
model = "deepseek-v4-flash"
enabled = false
```

`provider.conf [defaults]` may specify both `provider` and `model`. Defaults
are used as a bootstrap and fallback for missing workflow assignments. Runtime
resolution is deterministic:

1. use the workflow's explicit `provider` and `model`;
2. otherwise use `provider.conf [defaults].provider` and
   `provider.conf [defaults].model`;
3. otherwise fail closed with a clear configuration error.

When a workflow is edited and saved through `hiero config`, the saved workflow
should become explicit. Defaults improve setup ergonomics; they do not replace
clear persisted workflow assignments.

## Configuration Responsibilities

`provider.conf` owns:

- provider profile id, from the TOML table name;
- human display name;
- provider protocol type, such as `openai`, `google`, `anthropic`, or `ollama`;
- base URL;
- API key or absence of one;
- provider-level timeout and future endpoint-level settings;
- default provider and model for new or incomplete workflow assignments.

`dream.conf` owns:

- dreaming/autostart thresholds and caps;
- prompts and dreaming-specific controls;
- workflow assignments by workflow id;
- workflow-specific model choice;
- workflow enabled/disabled state.

No runtime database or translation workspace file owns provider credentials.

## UI Implications

`hiero config` should present provider profiles and workflow assignments as
separate editing concerns:

1. A provider catalog area edits `provider.conf`: create, rename, delete, test,
   and list model suggestions for configured provider profiles.
2. A workflow assignment area edits `dream.conf`: choose a named provider
   profile and model per workflow.

Provider checks and model suggestions operate on provider profiles from
`provider.conf`. Workflow validation checks that enabled workflows resolve to a
known provider profile and a non-empty model, either explicitly or through
defaults.

Displayed API keys must remain redacted. Saving a redacted marker such as
`***` must preserve the existing secret. Supplying a new key must update
`provider.conf`, not `dream.conf`.

## Migration

The current `dream.conf.providers` shape is deprecated and must be migrated.
Migration should:

1. read existing provider profiles from `dream.conf.providers`;
2. write them to `provider.conf` under stable profile ids;
3. preserve API keys exactly;
4. update `dream.conf.workflows.*.provider` so workflow assignments refer to
   the migrated profile ids;
5. remove provider profile blocks from `dream.conf` after a successful
   migration.

If migration cannot determine a safe profile id or would overwrite a different
existing profile in `provider.conf`, it must fail closed with a diagnostic
instead of silently changing credentials or workflow routing.

## Consequences

This split makes provider access reusable by multiple workflows and future LLM
features without duplicating secrets. It also makes the strict configuration
model easier to validate: provider profiles are endpoint templates, workflows
are consumers.

The change requires a compatibility and migration pass because existing
installations may already have `dream.conf.providers`. During implementation,
tests must cover both fresh configuration and migration from the current
combined format.

ADR 0001 remains in force for plaintext local configuration and redaction. This
ADR narrows ownership: provider credentials move from `dream.conf` to
`provider.conf`; they remain local plaintext secrets with strict redaction in
UI, JSON bridge payloads, logs, doctor output, provider checks, and audit data.
