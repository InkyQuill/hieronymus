# Store Dreaming Secrets In Plaintext Dream Config

Hieronymus stores dreaming workflow provider configuration in
`~/.config/hieronymus/dream.conf`, scoped to dreaming provider, trigger, prompt,
and workflow-cap settings. API keys are stored as plaintext in that file because
the project is local-first and we do not want to add an encryption/key-management
layer yet. Discovered provider model lists are refreshable cache data and live
separately in `~/.config/hieronymus/llmcache.tmp`. The admin UI, JSON bridge,
logs, provider checks, and audit records must still redact API key values and
expose only secret presence/status.
