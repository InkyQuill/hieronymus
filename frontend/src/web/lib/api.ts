import type {
  AdminDashboard,
  AdminActionBody,
  AdminActionResult,
  AdminSnapshot,
  DreamSettings,
  IngestSettings,
  ModelCache,
  ProviderDraft,
  ProviderCheck,
  ProviderProfile,
  ReleaseSettings,
} from "./types";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: "same-origin",
    headers: { "Content-Type": "application/json", ...(init?.headers ?? {}) },
    ...init,
  });
  const payload = (await response.json()) as T & { error?: string };
  if (!response.ok || payload.error)
    throw new Error(payload.error || "Request failed");
  return payload;
}

export async function listProviders(): Promise<ProviderProfile[]> {
  return (await request<{ providers: ProviderProfile[] }>("/api/providers"))
    .providers;
}

export async function saveProvider(
  provider: ProviderDraft,
): Promise<ProviderProfile> {
  return (
    await request<{ provider: ProviderProfile }>("/api/providers", {
      method: "POST",
      body: JSON.stringify({ provider }),
    })
  ).provider;
}

export async function deleteProvider(providerId: string): Promise<void> {
  await request(`/api/providers/${encodeURIComponent(providerId)}`, {
    method: "DELETE",
  });
}

export async function refreshModels(providerId: string): Promise<string[]> {
  return (
    await request<{ models: string[] }>(
      `/api/providers/${encodeURIComponent(providerId)}/models`,
    )
  ).models;
}

export async function checkProvider(
  providerId: string,
): Promise<ProviderCheck> {
  return (
    await request<{ check: ProviderCheck }>(
      `/api/providers/${encodeURIComponent(providerId)}/check`,
      { method: "POST", body: "{}" },
    )
  ).check;
}

export async function loadDreamSettings(): Promise<{
  dream: DreamSettings;
  providers: ProviderProfile[];
  model_cache: ModelCache;
  default_prompts?: Record<string, string>;
}> {
  return request("/api/settings/dream");
}

export async function saveDreamSettings(
  dream: DreamSettings,
): Promise<DreamSettings> {
  return (
    await request<{ dream: DreamSettings }>("/api/settings/dream", {
      method: "POST",
      body: JSON.stringify({ dream }),
    })
  ).dream;
}

export async function loadIngestSettings(): Promise<IngestSettings> {
  return (await request<{ ingest: IngestSettings }>("/api/settings/ingest"))
    .ingest;
}

export async function saveIngestSettings(
  ingest: IngestSettings,
): Promise<IngestSettings> {
  return (
    await request<{ ingest: IngestSettings }>("/api/settings/ingest", {
      method: "POST",
      body: JSON.stringify({ ingest }),
    })
  ).ingest;
}

export async function loadReleaseSettings(): Promise<ReleaseSettings> {
  return (await request<{ release: ReleaseSettings }>("/api/settings/release"))
    .release;
}

export async function saveReleaseSettings(
  release: ReleaseSettings,
): Promise<ReleaseSettings> {
  return (
    await request<{ release: ReleaseSettings }>("/api/settings/release", {
      method: "POST",
      body: JSON.stringify({ release }),
    })
  ).release;
}

export async function loadAdminDashboard(): Promise<AdminDashboard> {
  return request("/api/admin/dashboard");
}

export async function loadAdminSnapshot(
  view: string,
  selectedId?: string | number,
  series?: string,
  paging?: { limit: number; offset: number },
): Promise<AdminSnapshot> {
  const query = new URLSearchParams({ view });
  if (paging) {
    query.set("limit", String(paging.limit));
    query.set("offset", String(paging.offset));
  }
  if (series) query.set("series", series);
  if (selectedId !== undefined) query.set("selected_id", String(selectedId));
  return request(`/api/admin/snapshot?${query}`);
}

export async function runAdminAction(
  action: string,
  body: AdminActionBody,
): Promise<AdminActionResult> {
  return request(`/api/admin/actions/${encodeURIComponent(action)}`, {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export async function startAdminDreaming(): Promise<{
  started: boolean;
  status: string;
}> {
  return request("/api/admin/actions/run_manual_dreaming", {
    method: "POST",
    body: "{}",
  });
}

export interface AgentConnection {
  id: string;
  name: string;
  instructions: string;
}
export async function prepareAgentConnection(): Promise<AgentConnection[]> {
  return (
    await request<{ agents: AgentConnection[] }>("/api/agents/prepare", {
      method: "POST",
      body: "{}",
    })
  ).agents;
}

export function loadComparisonSettings(): Promise<{
  comparison: import("./types").ComparisonState;
  providers: ProviderProfile[];
}> {
  return request("/api/settings/comparison");
}
export async function saveComparisonSettings(
  comparison: import("./types").ComparisonSettings,
): Promise<import("./types").ComparisonState> {
  return (
    await request<{ comparison: import("./types").ComparisonState }>(
      "/api/settings/comparison",
      { method: "POST", body: JSON.stringify({ comparison }) },
    )
  ).comparison;
}

export async function loadRelevanceSettings(): Promise<
  import("./types").RelevanceSettings
> {
  const result = await request<{
    relevance: import("./types").RelevanceSettings;
    error: string;
  }>("/api/settings/relevance");
  if (result.error) throw new Error(result.error);
  return result.relevance;
}
export async function saveRelevanceSettings(
  relevance: import("./types").RelevanceDraft,
): Promise<import("./types").RelevanceSettings> {
  return (
    await request<{ relevance: import("./types").RelevanceSettings }>(
      "/api/settings/relevance",
      {
        method: "POST",
        body: JSON.stringify({ relevance }),
      },
    )
  ).relevance;
}

export function loadVersion(): Promise<{ server_version: string }> {
  return request("/api/version");
}
