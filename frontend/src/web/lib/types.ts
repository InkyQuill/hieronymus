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

/// One entry of the admin command catalog (`ADMIN_COMMANDS` on the daemon).
/// The dashboard payload carries these as `command_options`; the console
/// derives the per-view action buttons and their selection requirement from
/// this list rather than a hand-maintained frontend map.
export type AdminCommand = {
  id: string;
  label: string;
  hint: string;
  key: string;
  group: string;
  views: string[];
  requires_selection: boolean;
};

export type AdminDashboard = {
  header: { product: string; version: string; tagline: string };
  stats: Record<string, number>;
  views: string[];
  command_options?: AdminCommand[];
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
    /// Active filter labels for the view (empty today; W2 keeps the key so
    /// the frontend contract already carries it).
    filters?: string[];
  };
};

/// Paging and scope inputs for `GET /api/admin/snapshot`. All optional: the
/// daemon defaults to a bounded page of the current view with no scope.
export type AdminSnapshotQuery = {
  view: string;
  selected_id?: string | number;
  /// Bounded page size; the daemon clamps this to its own maximum.
  limit?: number;
  offset?: number;
  /// Series slug the projection is scoped to; foreign-context rows are
  /// filtered out (global concepts/rows still appear).
  series?: string;
};

/// One part of a `split_crystal` request: a plain string, or an explicit
/// `{title, text}` pair.
export type AdminSplitPart = string | { title?: string; text?: string };

/// The per-action request body for `POST /api/admin/actions/{action}`. Every
/// field is optional here; the daemon validates the typed DTO for the named
/// action (`validate_action_request` / `run_action`). The actor is NOT a
/// field — it comes from the authenticated session.
export type AdminActionBody = {
  id?: string | number;
  ids?: Array<string | number>;
  confirmed?: boolean;
  view?: string;
  /// add_memory / edit_memory / merge_selected
  text?: string;
  title?: string;
  /// add_memory
  series?: string;
  source_language?: string;
  target_language?: string;
  crystal_type?: string;
  /// approve_proposal / reject_proposal
  reason?: string;
  /// reinforce_crystal / decay_crystal
  evidence?: string;
  /// inspect_recall_reasons
  recall_id?: string;
  /// review_dream_output
  run_id?: string | number;
  /// run_manual_dreaming
  all?: boolean;
  /// split_crystal
  parts?: AdminSplitPart[];
};

export type AdminProvenance = {
  title: string;
  sources: Array<Record<string, string>>;
};

export type AdminActionResult = {
  result: { message: string } & Record<string, unknown>;
  snapshot: AdminSnapshot["snapshot"];
  /// Present only for the read-only inspection actions.
  provenance?: AdminProvenance;
  reasons?: Array<Record<string, string>>;
  review?: Record<string, unknown>;
  run?: Record<string, unknown>;
};
