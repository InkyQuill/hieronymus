import type {
  AdminDashboard,
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
): Promise<AdminSnapshot> {
  const query = new URLSearchParams({ view });
  if (selectedId !== undefined) query.set("selected_id", String(selectedId));
  return request(`/api/admin/snapshot?${query}`);
}

export async function runAdminAction(
  action: string,
  params: { id: string | number; confirmed?: boolean },
): Promise<AdminActionResult> {
  return request(`/api/admin/actions/${encodeURIComponent(action)}`, {
    method: "POST",
    body: JSON.stringify(params),
  });
}

export async function startAdminDreaming(): Promise<
  Omit<AdminActionResult, "result"> & { result: { id: number; status: string } }
> {
  return runAdminAction("run_manual_dreaming", { id: 0 });
}
