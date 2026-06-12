import { describe, expect, it } from "bun:test";
import {
  AdminBootstrapSchema,
  AdminSnapshotSchema,
  ConfigBootstrapSchema,
  RpcResponseSchema,
} from "./schema.js";

describe("runtime schemas", () => {
  it.each([
    ["openai", "OpenAI compatible"],
    ["gemini", "Gemini"],
    ["anthropic", "Anthropic"],
  ] as const)(
    "parses config bootstrap payload for %s provider selector",
    (provider, displayName) => {
      const payload = ConfigBootstrapSchema.parse({
        config_paths: {
          config_root: "/tmp/h",
          settings_path: "/tmp/h/settings.toml",
          database_path: "/tmp/h/hieronymus.sqlite3",
        },
        provider_choices: [
          {
            name: provider,
            display_name: displayName,
            requires_api_key: true,
            supports_api_path: true,
          },
        ],
        selected_provider: provider,
        draft: { dreaming: { active_provider: provider }, providers: {} },
        form_values: { provider: { model: "gpt-4.1-mini" }, dreaming: {} },
        validation: { ok: true, errors: [] },
        suggestions: {
          provider,
          models: ["gpt-4.1-mini"],
          source: "defaults",
          error: "",
        },
        detail: {
          title: `${provider} dreaming provider`,
          fields: [],
          errors: [],
        },
      });

      expect(payload.selected_provider).toBe(provider);
    },
  );

  it("parses the current Python config bootstrap payload shape", () => {
    const payload = ConfigBootstrapSchema.parse({
      config_paths: {
        data_root: "/tmp/hieronymus",
        config_root: "/tmp/hieronymus/config",
        settings_path: "/tmp/hieronymus/config/settings.toml",
      },
      provider_choices: [
        {
          display_name: "OpenAI compatible",
          name: "openai",
          requires_api_key: true,
          supports_api_path: true,
        },
        {
          display_name: "Gemini",
          name: "gemini",
          requires_api_key: true,
          supports_api_path: false,
        },
        {
          display_name: "Anthropic",
          name: "anthropic",
          requires_api_key: true,
          supports_api_path: false,
        },
      ],
      selected_provider: "openai",
      draft: { dreaming: { active_provider: "openai" }, providers: {} },
      form_values: { provider: {}, dreaming: {} },
      validation: { ok: true, errors: [] },
      check_result: {},
      suggestions: {},
      detail: "",
    });

    expect(payload.suggestions).toEqual({});
    expect(payload.detail).toBe("");
    expect(payload.provider_choices[0].requires_api_key).toBe(true);
  });

  it("accepts an empty config detail payload", () => {
    const payload = ConfigBootstrapSchema.parse({
      config_paths: {
        settings_path: "/tmp/hieronymus/config/settings.toml",
      },
      provider_choices: [
        {
          display_name: "OpenAI compatible",
          name: "openai",
          requires_api_key: true,
          supports_api_path: true,
        },
      ],
      selected_provider: "openai",
      draft: { dreaming: { active_provider: "openai" }, providers: {} },
      form_values: { provider: {}, dreaming: {} },
      validation: { ok: true, errors: [] },
      suggestions: {},
      detail: {},
    });

    expect(payload.detail).toEqual({});
  });

  it("defaults missing model suggestions to an empty payload", () => {
    const payload = ConfigBootstrapSchema.parse({
      config_paths: {
        settings_path: "/tmp/hieronymus/config/settings.toml",
      },
      provider_choices: [
        {
          display_name: "OpenAI compatible",
          name: "openai",
          requires_api_key: true,
          supports_api_path: true,
        },
      ],
      selected_provider: "openai",
      draft: { dreaming: { active_provider: "openai" }, providers: {} },
      form_values: { provider: {}, dreaming: {} },
      validation: { ok: true, errors: [] },
      detail: {},
    });

    expect(payload.suggestions).toEqual({});
    expect(payload.release).toEqual({
      update_channel: "stable",
      update_target: "latest",
    });
    expect(payload.ingest).toEqual({ short_memory: {}, learn: {} });
    expect(payload.draft.release).toEqual({});
    expect(payload.form_values.release).toEqual({});
  });

  it("parses present ingest config payloads with numeric passthrough fields", () => {
    const payload = ConfigBootstrapSchema.parse({
      config_paths: {
        settings_path: "/tmp/hieronymus/config/settings.toml",
      },
      provider_choices: [
        {
          display_name: "OpenAI compatible",
          name: "openai",
          requires_api_key: true,
          supports_api_path: true,
        },
      ],
      selected_provider: "openai",
      draft: { dreaming: { active_provider: "openai" }, providers: {} },
      form_values: { provider: {}, dreaming: {} },
      ingest: {
        short_memory: {
          warning_sentence_count: 6,
          rejection_sentence_count: 30,
        },
        learn: { max_block_chars: 1200 },
        source: "defaults",
      },
      validation: { ok: true, errors: [] },
      detail: {},
    });

    expect(payload.ingest.short_memory.warning_sentence_count).toBe(6);
    expect(payload.ingest.learn.max_block_chars).toBe(1200);
    expect(payload.ingest.source).toBe("defaults");
  });

  it("rejects config provider choices outside supported families", () => {
    expect(() =>
      ConfigBootstrapSchema.parse({
        config_paths: {},
        provider_choices: [
          {
            name: "deterministic",
            display_name: "Deterministic",
            requires_api_key: false,
            supports_api_path: false,
          },
        ],
        selected_provider: "deterministic",
        draft: { dreaming: {}, providers: {} },
        form_values: { provider: {}, dreaming: {} },
        validation: { ok: true, errors: [] },
        suggestions: {
          provider: "deterministic",
          models: [],
          source: "defaults",
          error: "",
        },
        detail: { title: "", fields: [], errors: [] },
      }),
    ).toThrow();
  });

  it("parses admin snapshots", () => {
    const snapshot = AdminSnapshotSchema.parse({
      view: "Crystals",
      rows: [],
      selected: null,
      detail: { title: "Empty", subtitle: "", body: "", fields: [] },
      filters: [],
    });

    expect(snapshot.view).toBe("Crystals");
  });

  it("parses admin bootstrap status and config contracts", () => {
    const payload = AdminBootstrapSchema.parse({
      views: ["Crystals", "Short-Term Sessions", "Dream Runs"],
      default_view: "Crystals",
      header: {
        product: "Hieronymus",
        version: "0.1.0",
        tagline: "Local translation memory.",
        logo: { text: "H", name: "feather", alt: "Hieronymus feather logo" },
      },
      stats: { series: 1, crystals: 1 },
      service: { running: false },
      snapshot: {
        view: "Crystals",
        rows: [],
        selected: null,
        detail: { title: "Empty", subtitle: "", body: "", fields: [] },
        filters: [],
      },
      short_term_status: {
        pending_count: 4,
        min_pending_short_term_memories: 20,
        max_pending_short_term_memories: 200,
        urgent: false,
        drain_in_progress: true,
        drain_completed: 6,
        drain_remaining: 4,
        drain_total: 10,
        drain_progress: 0.6,
      },
      dream_status: {
        state: "WORKING",
        current_phase: "maintenance",
        progress: 0.75,
        run_id: 12,
        cycle_id: 4,
        owner: "admin",
        started_at: "2026-06-10T00:00:00+00:00",
      },
      config_editor: {
        providers: { anthropic: { provider_type: "anthropic" } },
        workflows: {
          crystallization: { provider: "anthropic", model: "claude" },
        },
        prompts: { general: "Translate with continuity." },
        thresholds: { max_pending_short_term_memories: 200 },
        model_cache_warnings: [
          {
            workflow: "crystallization",
            provider: "anthropic",
            code: "model_cache_missing",
            message: "model cache has not been fetched for provider",
          },
        ],
      },
    });

    expect(payload.header.logo.alt).toBe("Hieronymus feather logo");
    expect(payload.short_term_status.drain_progress).toBe(0.6);
    expect(payload.dream_status.current_phase).toBe("maintenance");
    expect(payload.config_editor.model_cache_warnings[0].code).toBe(
      "model_cache_missing",
    );
  });

  it("parses success and error envelopes", () => {
    expect(
      RpcResponseSchema.parse({ id: "1", ok: true, result: { ready: true } })
        .ok,
    ).toBe(true);
    expect(
      RpcResponseSchema.parse({
        id: "1",
        ok: false,
        error: { code: "validation_error", message: "text must not be empty" },
      }).ok,
    ).toBe(false);
  });
});
