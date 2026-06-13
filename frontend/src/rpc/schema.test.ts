import { describe, expect, it } from "bun:test";
import {
  AdminBootstrapSchema,
  AdminSnapshotSchema,
  ConfigBootstrapSchema,
  RpcResponseSchema,
} from "./schema.js";

function configDraft(provider: string = "openai") {
  return {
    dream: {
      dreaming: {},
      providers: {},
      workflows: {
        crystallization: {
          provider,
          model: provider === "gemini" ? "gemini-2.5-flash" : "gpt-4.1-mini",
          enabled: true,
        },
      },
    },
    ingest: { short_memory: {}, learn: {} },
    release: { update_channel: "stable" },
    dreaming: { active_provider: provider },
    providers: {},
    workflows: {},
  };
}

function configPaths() {
  return {
    data_root: "/tmp/hieronymus",
    config_root: "/tmp/hieronymus/config",
    dream_config_path: "/tmp/hieronymus/config/dream.conf",
    ingest_config_path: "/tmp/hieronymus/config/ingest.conf",
    release_config_path: "/tmp/hieronymus/config/release.conf",
  };
}

function configPayload(
  provider: "openai" | "gemini" | "anthropic" = "openai",
  overrides: Record<string, unknown> = {},
) {
  return {
    config_paths: configPaths(),
    provider_choices: [
      {
        name: provider,
        display_name:
          provider === "openai"
            ? "OpenAI compatible"
            : provider === "gemini"
              ? "Gemini"
              : "Anthropic",
        requires_api_key: true,
        supports_api_path: provider === "openai",
      },
    ],
    selected_provider: provider,
    draft: configDraft(provider),
    form_values: {
      provider: {
        model: provider === "gemini" ? "gemini-2.5-flash" : "gpt-4.1-mini",
      },
      dreaming: {},
      ingest: {},
      release: {},
    },
    validation: { ok: true, errors: [] },
    suggestions: {},
    detail: {},
    ...overrides,
  };
}

describe("runtime schemas", () => {
  it.each([
    ["openai", "OpenAI compatible"],
    ["gemini", "Gemini"],
    ["anthropic", "Anthropic"],
  ] as const)(
    "parses config bootstrap payload for %s provider selector",
    (provider, displayName) => {
      const payload = ConfigBootstrapSchema.parse({
        config_paths: configPaths(),
        provider_choices: [
          {
            name: provider,
            display_name: displayName,
            requires_api_key: true,
            supports_api_path: true,
          },
        ],
        selected_provider: provider,
        draft: configDraft(provider),
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
        dream_config_path: "/tmp/hieronymus/config/dream.conf",
        ingest_config_path: "/tmp/hieronymus/config/ingest.conf",
        release_config_path: "/tmp/hieronymus/config/release.conf",
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
      draft: configDraft(),
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

  it("parses Python-owned config form schema", () => {
    const payload = ConfigBootstrapSchema.parse(
      configPayload("openai", {
        form_schema: {
          groups: [
            {
              id: "provider",
              label: "Provider",
              description:
                "Connection settings for the selected dream provider.",
            },
          ],
          fields: [
            {
              key: "provider.api_key",
              group: "provider",
              label: "API Key",
              hint: "Stored as plaintext in dream.conf and redacted in UI payloads.",
              placeholder: "stored in dream.conf",
              type: "secret",
              redacted: true,
            },
            {
              key: "release.update_channel",
              group: "release",
              label: "Update Channel",
              hint: "Managed install update channel.",
              placeholder: "",
              type: "choice",
              choices: ["stable", "dev"],
              default: "stable",
              redacted: false,
            },
          ],
        },
      }),
    );

    expect(payload.form_schema).toBeDefined();
    if (!payload.form_schema) {
      throw new Error("form_schema should be defaulted");
    }
    expect(payload.form_schema.fields[0].type).toBe("secret");
    expect(payload.form_schema.fields[1].choices).toEqual(["stable", "dev"]);
  });

  it("defaults missing config form schema to empty groups and fields", () => {
    const payload = ConfigBootstrapSchema.parse(configPayload());

    expect(payload.form_schema).toEqual({ groups: [], fields: [] });
  });

  it("parses config field validation errors", () => {
    const payload = ConfigBootstrapSchema.parse(
      configPayload("openai", {
        validation: {
          ok: false,
          errors: ["providers.openai.timeout_seconds must be greater than 0"],
          field_errors: {
            "provider.timeout_seconds": [
              "providers.openai.timeout_seconds must be greater than 0",
            ],
          },
        },
      }),
    );

    expect(payload.validation.field_errors["provider.timeout_seconds"]).toEqual(
      ["providers.openai.timeout_seconds must be greater than 0"],
    );
  });

  it("applies Python-owned config form field defaults", () => {
    const payload = ConfigBootstrapSchema.parse(
      configPayload("openai", {
        form_schema: {
          groups: [{ id: "provider", label: "Provider" }],
          fields: [
            {
              key: "provider.model",
              group: "provider",
              label: "Model",
              type: "text",
            },
          ],
        },
      }),
    );

    expect(payload.form_schema).toBeDefined();
    if (!payload.form_schema) {
      throw new Error("form_schema should be defaulted");
    }
    expect(payload.form_schema.groups[0].description).toBe("");
    expect(payload.form_schema.fields[0]).toMatchObject({
      choices: [],
      default: "",
      hint: "",
      placeholder: "",
      redacted: false,
    });
    expect(payload.form_schema.fields[0].minimum).toBeUndefined();
  });

  it("parses Python-owned config form field minimum values", () => {
    const payload = ConfigBootstrapSchema.parse(
      configPayload("openai", {
        form_schema: {
          groups: [{ id: "provider", label: "Provider" }],
          fields: [
            {
              key: "provider.timeout_seconds",
              group: "provider",
              label: "Timeout",
              type: "number",
              minimum: 1,
            },
          ],
        },
      }),
    );

    expect(payload.form_schema.fields[0].minimum).toBe(1);
  });

  it("rejects invalid config form field types", () => {
    expect(() =>
      ConfigBootstrapSchema.parse(
        configPayload("openai", {
          form_schema: {
            groups: [{ id: "provider", label: "Provider" }],
            fields: [
              {
                key: "provider.api_key",
                group: "provider",
                label: "API Key",
                type: "password",
              },
            ],
          },
        }),
      ),
    ).toThrow();
  });

  it("rejects null config form schema payloads", () => {
    expect(() =>
      ConfigBootstrapSchema.parse(
        configPayload("openai", {
          form_schema: null,
        }),
      ),
    ).toThrow();
  });

  it("accepts an empty config detail payload", () => {
    const payload = ConfigBootstrapSchema.parse({
      config_paths: configPaths(),
      provider_choices: [
        {
          display_name: "OpenAI compatible",
          name: "openai",
          requires_api_key: true,
          supports_api_path: true,
        },
      ],
      selected_provider: "openai",
      draft: configDraft(),
      form_values: { provider: {}, dreaming: {} },
      validation: { ok: true, errors: [] },
      suggestions: {},
      detail: {},
    });

    expect(payload.detail).toEqual({});
  });

  it("defaults missing model suggestions to an empty payload", () => {
    const payload = ConfigBootstrapSchema.parse({
      config_paths: configPaths(),
      provider_choices: [
        {
          display_name: "OpenAI compatible",
          name: "openai",
          requires_api_key: true,
          supports_api_path: true,
        },
      ],
      selected_provider: "openai",
      draft: configDraft(),
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
    expect(payload.draft.release).toEqual({ update_channel: "stable" });
    expect(payload.form_values.release).toEqual({});
  });

  it("requires real config draft sections", () => {
    expect(() =>
      ConfigBootstrapSchema.parse({
        config_paths: configPaths(),
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
      }),
    ).toThrow();
  });

  it("parses present ingest config payloads with numeric passthrough fields", () => {
    const payload = ConfigBootstrapSchema.parse({
      config_paths: configPaths(),
      provider_choices: [
        {
          display_name: "OpenAI compatible",
          name: "openai",
          requires_api_key: true,
          supports_api_path: true,
        },
      ],
      selected_provider: "openai",
      draft: configDraft(),
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
        config_paths: configPaths(),
        provider_choices: [
          {
            name: "deterministic",
            display_name: "Deterministic",
            requires_api_key: false,
            supports_api_path: false,
          },
        ],
        selected_provider: "deterministic",
        draft: configDraft("deterministic"),
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
