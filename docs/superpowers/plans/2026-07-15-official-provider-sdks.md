# Official Provider SDK Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hand-written HTTP integration for dream providers with small typed adapters over the official OpenAI, Google Gen AI, Anthropic, and Ollama Python SDKs. A configured credential must enable live model discovery for the Config TUI; cached results and reliable fallback suggestions remain available when discovery cannot run.

**Architecture:** Keep `ProviderRegistry`, its cache contract, and the public provider configuration format as the orchestration layer. Introduce a provider-client protocol plus one adapter per SDK in `dream_providers.py` (or a dedicated adjacent module if it stays focused). The registry and dream providers depend only on that protocol; a factory supplies production SDK adapters or deterministic fakes in tests. The persisted Google type remains `google` and is never treated as an unsupported `gemini` profile.

**Tech Stack:** Python 3.12, `uv`, official `openai`, `google-genai`, `anthropic`, and `ollama` packages, pytest, Ruff.

## Global constraints

- Do not make raw HTTP requests to provider APIs or reimplement provider authentication, pagination, retries, or serialization.
- Use the SDK model-list APIs whenever a profile has a usable key. For Ollama, also discover models from its configured local endpoint without a key; include the key when configured.
- Preserve model-list cache identity, TTL, stale/error behaviour, profile defaults, output schema, and secret redaction.
- Maintain support for OpenAI-compatible endpoints through the official `OpenAI(base_url=...)` client.
- Keep generation deterministic where the existing provider contract requires it; parse and validate generated output before returning it.
- Translate SDK exceptions to the existing user-facing error vocabulary without exposing credentials or full SDK request details.

---

## File structure

| File | Change | Responsibility |
|---|---|---|
| `pyproject.toml` / `uv.lock` | Modify | Add and lock official runtime SDK dependencies. |
| `src/hieronymus/dream_providers.py` | Modify | SDK adapter protocol/factory; discovery, health checks, and dream generation through adapters; remove `urllib` transport. |
| `src/hieronymus/provider_config.py` | Modify | Normalize legacy persisted `gemini` profiles to canonical `google`. |
| `src/hieronymus/tui_bridge/config_api.py` | Verify/minimal modify | Keep config check and explicit refresh wired to live suggestions and surface a safe discovery error. |
| `tests/test_dream_providers.py` | Modify | Replace request-envelope assertions with typed SDK-adapter contract tests. |
| `tests/test_provider_config.py` | Modify | Cover canonical Google type and legacy migration. |
| `tests/test_tui_bridge_config.py` | Modify | Cover model suggestions after a successful keyed provider check. |
| `README.md` and/or `docs/` provider reference | Modify | State installed SDK-backed providers and discovery/fallback semantics. |

## Task 1: Add the official SDKs and a testable adapter boundary

**Files:** `pyproject.toml`, `uv.lock`, `src/hieronymus/dream_providers.py`, `tests/test_dream_providers.py`

- [ ] Add direct runtime dependencies `openai`, `google-genai`, `anthropic`, and `ollama`; run `uv lock` and confirm all import under the project environment.
- [ ] Replace `HTTPResponse`, `HTTPTransport`, and `UrllibTransport` with a small `ProviderClient` protocol that has only the project operations: `list_models()`, `check(model)`, and `generate_dream(...)`.
- [ ] Add an injectable `ProviderClientFactory` to `ProviderRegistry` and `resolve_provider`. Production construction must be lazy so importing or using a deterministic provider does not require network activity or credentials.
- [ ] Convert the existing `FakeTransport` test helper into typed fake clients/factory, recording semantic calls rather than URL, header, or JSON body internals.

**Acceptance tests:** adapter construction selects the correct client for `openai`, `google`, `anthropic`, and `ollama`; unknown types remain rejected; no source import of `urllib.request` remains.

**Commit:** `refactor: introduce provider sdk adapter boundary`

## Task 2: Implement SDK-native model discovery and keyed health checks

**Files:** `src/hieronymus/dream_providers.py`, `tests/test_dream_providers.py`

- [ ] Implement `OpenAIProviderClient` with `OpenAI(api_key=..., base_url=..., timeout=...)`. Use `client.models.list()` and collect model IDs; use the SDK’s generation API for the lightweight check.
- [ ] Implement `GoogleProviderClient` with `from google import genai` and `genai.Client(api_key=...)` (including configured endpoint/HTTP options where supported). Use `client.models.list()`, retaining only models whose `supported_actions` include `generateContent`; check via `client.models.generate_content`.
- [ ] Implement `AnthropicProviderClient` with `Anthropic(api_key=..., base_url=..., timeout=...)`. Use the SDK’s auto-paginating `client.models.list()` iterator, then `client.messages.create()` for a bounded health check.
- [ ] Implement `OllamaProviderClient` with `ollama.Client(host=..., headers=...)`. Use `client.list()` and `client.chat()` for a health check. A configured key is sent through the SDK client headers; an unauthenticated local endpoint still lists its installed models.
- [ ] Make `ProviderRegistry.list_profile_model_suggestions()` call the adapter on cache miss for every remote type, including Anthropic and Ollama. Preserve current default suggestions only for no-key/non-queryable profiles and provider/SDK failures, with the existing cache/error metadata.
- [ ] Preserve list ordering, deduplication, configured-model inclusion, and model-cache invalidation when endpoint or key identity changes. Convert SDK exceptions into current non-secret-bearing error messages.

**Acceptance tests:** each keyed provider yields SDK-returned models; Google excludes non-generative models; Anthropic pagination is fully consumed; Ollama’s local model list works without a key; cache hit does not call the adapter and changing key/URL does.

**Commit:** `feat: discover provider models through official sdks`

## Task 3: Route dream generation through native structured-output features

**Files:** `src/hieronymus/dream_providers.py`, `tests/test_dream_providers.py`

- [ ] Move the common dream JSON schema and response validation beside the adapter protocol, so every provider returns one validated project payload rather than an SDK response object.
- [ ] Use the official SDK’s structured output mode where supported: OpenAI Responses structured JSON schema, Google `GenerateContentConfig` JSON MIME type/schema, and Ollama `chat(format=<JSON schema>)`.
- [ ] Use `Anthropic.messages.create()` through the official SDK with the same explicit JSON contract and validate its text-block response; enable an official structured-output facility only when it is stable for the pinned SDK/API version.
- [ ] Preserve existing system prompts, temperature/max-token bounds, parsing failures, and deterministic provider behaviour. Do not silently accept malformed JSON or a schema mismatch.
- [ ] Ensure OpenAI-compatible profiles use the same adapter only for compatible generation features; if a custom endpoint rejects a structured-output option, return a clear provider error instead of falling back to raw HTTP.

**Acceptance tests:** each adapter receives the expected high-level generation options, valid responses produce the existing dream result, text/SDK errors are normalized, malformed output remains rejected, and no test asserts provider-specific HTTP headers or paths.

**Commit:** `refactor: generate dreams through official provider sdks`

## Task 4: Canonicalize Google profiles and wire Config TUI discovery end-to-end

**Files:** `src/hieronymus/provider_config.py`, `src/hieronymus/tui_bridge/config_api.py`, `tests/test_provider_config.py`, `tests/test_tui_bridge_config.py`

- [ ] Normalize the legacy persisted `type = "gemini"` spelling to `type = "google"` during load/migration before supported-type validation. Save only the canonical spelling.
- [ ] Keep the display name “Gemini” if already used by the UI, but pass canonical `google` to the SDK factory without the old runtime-only `gemini` conversion leak.
- [ ] Verify `check_provider` invokes discovery after a successful check and `model_suggestions` explicitly refreshes through the same cached registry path. Preserve the selected configured model if it is absent from the remote list.
- [ ] Return a concise, redacted discovery failure to the TUI while still rendering defaults/cached models; a failed list must not make an otherwise valid saved configuration unusable.

**Acceptance tests:** legacy Gemini config loads and saves as Google; a Google-keyed config check shows SDK-discovered models; discovery failure leaves the form usable and does not reveal the key.

**Commit:** `fix: canonicalize google provider configuration`

## Task 5: Remove legacy transport assumptions and document operational behaviour

**Files:** `src/hieronymus/dream_providers.py`, `tests/test_dream_providers.py`, `README.md` and/or provider documentation

- [ ] Remove dead raw-HTTP parsers, imports, transport injection parameters, and request-shape tests once all callers use the adapter factory.
- [ ] Ensure SDK timeouts, retries, and connection management are configured through each official client, with project timeout values preserved where their API supports them.
- [ ] Document the provider matrix: canonical type, official package/client, discovery trigger, fallback source, and custom-endpoint limitations.
- [ ] Add a regression test that scans the module’s production imports/usage boundary for no direct `urllib` provider request path, without coupling the test to third-party implementation details.

**Acceptance tests:** static type checks/import tests pass with all four SDKs installed; provider defaults still work offline; docs match config types and runtime behaviour.

**Commit:** `docs: document sdk-backed provider discovery`

## Task 6: Full verification and manual Config TUI smoke test

**Files:** all changed files

- [ ] Run focused provider, provider-config, and TUI bridge tests while implementing each task.
- [ ] Run the required full checks:

  ```bash
  uv run pytest
  uv run ruff check .
  uv run ruff format --check .
  bun run --cwd frontend test
  ```

- [ ] Run `uv tool install --force --reinstall .` and manually open `hiero config` with a non-production test profile/key or a local Ollama endpoint. Confirm model suggestions appear after provider check, no secret is printed, and cached/default suggestions remain usable when the endpoint is unavailable.
- [ ] Inspect the final diff for accidental transport code, credential literals, and unrelated untracked files; do not stage `.x-skills.local.yaml` or `docs/tui-design-improvements.md`.

**Commit:** no separate commit unless verification exposes a focused fix.

## Final review

- [ ] Compare each provider against the official SDK documentation: correct import/client, model-list method, generation method, authentication, timeout, and pagination.
- [ ] Confirm every persisted provider type is canonical (`openai`, `google`, `anthropic`, `ollama`) and that legacy `gemini` only exists in migration coverage.
- [ ] Confirm the registry never fetches models without a credential except for a configured local Ollama endpoint, and never replaces a configured model or fallback with an empty remote response.
- [ ] Confirm no raw request construction or secret-bearing exception leaks remain.
