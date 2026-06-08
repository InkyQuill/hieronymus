import {z} from 'zod';

export const ProviderNameSchema = z.enum(['openai', 'gemini', 'anthropic']);

export const RpcResponseSchema = z.discriminatedUnion('ok', [
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

export const AdminRowSchema = z.object({
  id: z.union([z.number(), z.string()]),
  kind: z.string(),
  label: z.string(),
  status: z.string(),
  scope: z.string(),
  language_pair: z.string(),
  quality_label: z.string().default(''),
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

export const ConfigBootstrapSchema = z.object({
  config_paths: z.record(z.string()),
  provider_choices: z.array(
    z.object({
      name: ProviderNameSchema,
      display_name: z.string(),
      supports_api_path: z.boolean(),
    }),
  ),
  selected_provider: ProviderNameSchema,
  draft: z.object({
    dreaming: z.record(z.unknown()),
    providers: z.record(z.record(z.unknown())),
  }),
  form_values: z.object({
    provider: z.record(z.string()),
    dreaming: z.record(z.string()),
  }),
  validation: z.object({
    ok: z.boolean(),
    errors: z.array(z.string()),
  }),
  suggestions: z.object({
    provider: ProviderNameSchema,
    models: z.array(z.string()),
    source: z.string(),
    error: z.string(),
  }),
  detail: z.object({
    title: z.string(),
    fields: z.array(z.tuple([z.string(), z.string()])),
    errors: z.array(z.string()),
  }),
});

export type ProviderName = z.infer<typeof ProviderNameSchema>;
export type AdminRow = z.infer<typeof AdminRowSchema>;
export type AdminSnapshot = z.infer<typeof AdminSnapshotSchema>;
export type ConfigBootstrap = z.infer<typeof ConfigBootstrapSchema>;
