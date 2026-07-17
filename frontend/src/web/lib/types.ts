export type ProviderProfile = {
  id: string;
  name: string;
  type: "openai" | "google" | "anthropic" | "ollama";
  url: string;
  key_configured: boolean;
  model: string;
  timeout_seconds: number;
};

export type ProviderDraft = {
  id: string;
  name: string;
  type: ProviderProfile["type"];
  url: string;
  key: string;
  timeout_seconds: string;
};

export type ProviderCheck = {
  ok: boolean;
  models: string[];
  source: string;
  error: string;
};

export type Workflow = {
  provider: string;
  model: string;
  enabled: boolean;
  max_records_per_pass: number;
};

export type DreamingValues = {
  enabled: boolean;
  schedule_interval_minutes: number;
  min_pending_short_term_memories: number;
  max_pending_short_term_memories: number;
  max_short_term_memories_per_cycle: number;
  not_enough_memories_cycle_threshold: number;
  max_changed_crystals_per_cycle: number;
  max_related_concepts_per_cycle: number;
  max_related_crystals_per_concept: number;
  max_total_affected_crystals: number;
  max_short_term_memories_per_run: number;
  max_long_term_records_affected_per_run: number;
  max_relation_records_per_pass: number;
  general_prompt: string;
};

export type DreamSettings = {
  dreaming: DreamingValues;
  workflows: Record<string, Workflow>;
};

export type ModelCache = {
  providers: Record<string, { models: string[] }>;
};

export type IngestSettings = {
  short_memory: {
    warning_sentence_count: number;
    rejection_sentence_count: number;
    warning_symbol_count: number;
    rejection_symbol_count: number;
  };
  learn: { max_block_chars: number };
};

export type ReleaseSettings = { update_channel: "stable" | "dev" };

export type AdminDashboard = {
  header: { product: string; version: string; tagline: string };
  stats: Record<string, number>;
  views: string[];
  short_term_status: Record<string, unknown>;
  dream_status: Record<string, unknown>;
};

export type AdminRow = {
  id: string | number;
  kind: string;
  label: string;
  status: string;
  scope: string;
  language_pair: string;
  quality_label: string;
  tags: string[];
};

export type AdminDetail = {
  title: string;
  subtitle: string;
  body: string;
  fields: Array<[string, string]>;
};

export type AdminSnapshot = {
  snapshot: {
    view: string;
    rows: AdminRow[];
    selected: AdminRow | null;
    detail: AdminDetail;
  };
};

export type AdminActionResult = {
  result: { message: string };
  snapshot: AdminSnapshot["snapshot"];
};
