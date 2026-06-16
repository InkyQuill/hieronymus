# Store Local Configuration In Plaintext Config Files

Status: Superseded in part by
[ADR 0007](0007-provider-catalog-and-workflow-assignments.md). The plaintext
local-secret decision remains current, but provider profiles and API keys are
now owned by `provider.conf`; `dream.conf` owns workflow assignments, prompts,
trigger settings, and caps.

Hieronymus is local-first alpha software, so user-editable configuration should
live in plaintext files under the configured data root rather than in
environment-only provider settings or an encryption/key-management layer.
Provider profiles and local API keys are stored in `provider.conf`; `dream.conf`
stores workflow assignments, prompts, trigger settings, and caps. Future
configuration files should follow the same plaintext model when they are needed;
`ingest.conf` owns global ingestion limits such as short-term memory
warning/rejection thresholds and Learn-style split limits.

Plaintext configuration is a deliberate UX choice: it is easier for a local user
to inspect, edit, back up, and reason about than hidden environment variable
state. Secret values may be stored in local config files, but the admin UI, JSON
bridge, logs, provider checks, doctor output, and audit records must redact
secret values and expose only safe status or presence information.

Refreshable cache data is not authoritative configuration. Provider model lists
remain cache data and live separately in `llmcache.tmp`.
