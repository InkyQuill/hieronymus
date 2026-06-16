import { z } from "zod";

export const ProviderNameSchema = z.string().min(1);
const ProviderCatalogDefaultProviderSchema = z.string();
const PositiveTimeoutStringSchema = z
  .string()
  .trim()
  .regex(/^(?:\d+\.?\d*|\.\d+)$/)
  .refine((value) => Number(value) > 0);
const TimeoutSecondsSchema = z
  .union([z.number().finite().positive(), PositiveTimeoutStringSchema])
  .default(30);

export const RpcResponseSchema = z.discriminatedUnion("ok", [
  z.object({
    id: z.string(),
    ok: z.literal(true),
    result: z.record(z.unknown()),
  }),
  z.object({
    id: z.string().nullable(),
    ok: z.literal(false),
    error: z.object({
      code: z.string(),
      message: z.string(),
    }),
  }),
]);

export const ModelSuggestionsSchema = z.union([
  z
    .object({
      provider: ProviderNameSchema,
      models: z.array(z.string()),
      source: z.string(),
      error: z.string(),
    })
    .passthrough(),
  z.object({}).strict(),
]);

export const ConfigDetailSchema = z.union([
  z.string(),
  z
    .object({
      title: z.string(),
      fields: z.array(z.tuple([z.string(), z.string()])),
      errors: z.array(z.string()),
    })
    .passthrough(),
  z.object({}).strict(),
]);

export const AdminRowSchema = z.object({
  id: z.union([z.number(), z.string()]),
  kind: z.string(),
  label: z.string(),
  status: z.string(),
  scope: z.string(),
  language_pair: z.string(),
  quality_label: z.string().default(""),
  tags: z.array(z.string()).default([]),
});

export const AdminDetailSchema = z.object({
  title: z.string(),
  subtitle: z.string(),
  body: z.string(),
  fields: z.array(z.tuple([z.string(), z.string()])).default([]),
});

export const AdminSnapshotSchema = z.object({
  view: z.string(),
  rows: z.array(AdminRowSchema),
  selected: AdminRowSchema.nullable(),
  detail: AdminDetailSchema,
  filters: z.array(z.string()),
});

export const AdminHeaderSchema = z.object({
  product: z.string(),
  version: z.string(),
  tagline: z.string(),
  logo: z
    .object({
      text: z.string(),
      name: z.string(),
      alt: z.string(),
    })
    .passthrough(),
});

export const AdminCommandSchema = z
  .object({
    id: z.string(),
    label: z.string(),
    hint: z.string(),
    key: z.string(),
    group: z.string(),
    views: z.array(z.string()),
    requires_selection: z.boolean(),
  })
  .passthrough();

export const AdminShortTermStatusSchema = z.object({
  pending_count: z.number(),
  min_pending_short_term_memories: z.number(),
  max_pending_short_term_memories: z.number(),
  urgent: z.boolean(),
  drain_in_progress: z.boolean().default(false),
  drain_completed: z.number().default(0),
  drain_remaining: z.number().default(0),
  drain_total: z.number().default(0),
  drain_progress: z.number().default(0),
});

export const AdminDreamStatusSchema = z
  .object({
    state: z.string(),
    current_phase: z.string(),
    progress: z.number(),
    run_id: z.number().nullable(),
    cycle_id: z.number().nullable(),
    owner: z.string(),
    started_at: z.string(),
  })
  .passthrough();

export const AdminConfigEditorSchema = z
  .object({
    config: z.record(z.unknown()).default({}),
    config_error: z.string().default(""),
    providers: z.record(z.record(z.unknown())),
    workflows: z.record(z.record(z.unknown())),
    prompts: z.record(z.string()),
    thresholds: z.record(z.number()),
    model_cache: z.record(z.unknown()).default({}),
    model_cache_warnings: z.array(z.record(z.string())),
  })
  .passthrough();

export const AdminBootstrapSchema = z.object({
  views: z.array(z.string()),
  default_view: z.string(),
  header: AdminHeaderSchema,
  stats: z.record(z.number()),
  service: z
    .object({
      running: z.boolean(),
    })
    .passthrough(),
  snapshot: AdminSnapshotSchema,
  short_term_status: AdminShortTermStatusSchema,
  dream_status: AdminDreamStatusSchema,
  config_editor: AdminConfigEditorSchema,
  command_options: z.array(AdminCommandSchema).default([]),
});

export const ConfigPathsSchema = z
  .object({
    dream_config_path: z.string(),
    provider_config_path: z.string().default(""),
    ingest_config_path: z.string(),
    release_config_path: z.string(),
  })
  .passthrough();

const ProviderCatalogProfileSchema = z
  .object({
    name: ProviderNameSchema.default(""),
    type: z.string().default(""),
    url: z.string().default(""),
    key: z.string().default(""),
    timeout_seconds: TimeoutSecondsSchema,
  })
  .passthrough();

const ProviderCatalogSchema = z
  .object({
    profiles: z.record(ProviderNameSchema, ProviderCatalogProfileSchema).default({}),
    defaults: z
      .object({
        provider: ProviderCatalogDefaultProviderSchema.default(""),
        model: z.string().default(""),
      })
      .passthrough()
      .default({ provider: "", model: "" }),
  })
  .passthrough()
  .default({ profiles: {}, defaults: { provider: "", model: "" } });

const ConfigFieldTypeSchema = z.enum([
  "text",
  "secret",
  "number",
  "toggle",
  "choice",
]);

const ConfigFormSectionSchema = z
  .object({
    id: z.string(),
    label: z.string(),
    description: z.string().default(""),
  })
  .passthrough();

const ConfigFormGroupSchema = z
  .object({
    id: z.string(),
    section: z.string().default(""),
    label: z.string(),
    description: z.string().default(""),
  })
  .passthrough();

const ConfigFormFieldSchema = z
  .object({
    key: z.string(),
    group: z.string(),
    section: z.string().default(""),
    label: z.string(),
    hint: z.string().default(""),
    placeholder: z.string().default(""),
    type: ConfigFieldTypeSchema,
    choices: z.array(z.string()).default([]),
    default: z.string().default(""),
    minimum: z.number().optional(),
    redacted: z.boolean().default(false),
  })
  .passthrough();

const ConfigFormSchemaSchema = z
  .object({
    sections: z.array(ConfigFormSectionSchema).default([]),
    groups: z.array(ConfigFormGroupSchema),
    fields: z.array(ConfigFormFieldSchema),
  })
  .passthrough();

export const ConfigBootstrapSchema = z
  .object({
    config_paths: ConfigPathsSchema,
    provider_choices: z.array(
      z
        .object({
          name: ProviderNameSchema,
          display_name: z.string(),
          requires_api_key: z.boolean(),
          supports_api_path: z.boolean(),
        })
        .passthrough(),
    ),
    selected_provider: ProviderNameSchema,
    draft: z.object({
      dream: z
        .object({
          dreaming: z.record(z.unknown()),
          providers: z.record(z.record(z.unknown())),
          workflows: z.record(z.record(z.unknown())),
        })
        .passthrough(),
      ingest: z
        .object({
          short_memory: z.record(z.number()),
          learn: z.record(z.number()),
        })
        .passthrough(),
      dreaming: z.record(z.unknown()),
      provider_catalog: ProviderCatalogSchema,
      providers: z.record(z.record(z.unknown())),
      workflows: z.record(z.record(z.unknown())).default({}),
      release: z.record(z.unknown()),
    }),
    form_values: z.object({
      provider: z.record(z.string()).default({}),
      provider_catalog: z.record(z.string()).default({}),
      workflows: z.record(z.string()).default({}),
      dreaming: z.record(z.string()),
      ingest: z.record(z.string()).default({}),
      release: z.record(z.string()).default({}),
    }),
    provider_catalog: ProviderCatalogSchema,
    release: z
      .object({
        update_channel: z.string(),
        update_target: z.string(),
      })
      .passthrough()
      .default({ update_channel: "stable", update_target: "latest" }),
    ingest: z
      .object({
        short_memory: z.record(z.number()),
        learn: z.record(z.number()),
      })
      .passthrough()
      .default({ short_memory: {}, learn: {} }),
    validation: z.object({
      ok: z.boolean(),
      errors: z.array(z.string()),
      field_errors: z.record(z.array(z.string())).default({}),
    }),
    check_result: z.record(z.unknown()).default({}),
    suggestions: ModelSuggestionsSchema.default({}),
    detail: ConfigDetailSchema,
    form_schema: ConfigFormSchemaSchema.default({
      sections: [],
      groups: [],
      fields: [],
    }),
  })
  .passthrough();

export type ProviderName = z.infer<typeof ProviderNameSchema>;
export type RpcResponse = z.infer<typeof RpcResponseSchema>;
export type ModelSuggestions = z.infer<typeof ModelSuggestionsSchema>;
export type ConfigDetail = z.infer<typeof ConfigDetailSchema>;
export type ConfigFormSection = z.infer<typeof ConfigFormSectionSchema>;
export type ConfigFormField = z.infer<typeof ConfigFormFieldSchema>;
export type ConfigFormGroup = z.infer<typeof ConfigFormGroupSchema>;
export type AdminRow = z.infer<typeof AdminRowSchema>;
export type AdminDetail = z.infer<typeof AdminDetailSchema>;
export type AdminSnapshot = z.infer<typeof AdminSnapshotSchema>;
export type AdminHeader = z.infer<typeof AdminHeaderSchema>;
export type AdminCommand = z.infer<typeof AdminCommandSchema>;
export type AdminShortTermStatus = z.infer<typeof AdminShortTermStatusSchema>;
export type AdminDreamStatus = z.infer<typeof AdminDreamStatusSchema>;
export type AdminConfigEditor = z.infer<typeof AdminConfigEditorSchema>;
export type AdminBootstrap = z.infer<typeof AdminBootstrapSchema>;
export type ConfigBootstrap = z.infer<typeof ConfigBootstrapSchema>;
