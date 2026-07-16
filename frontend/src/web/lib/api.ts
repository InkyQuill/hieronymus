import type { ProviderDraft, ProviderProfile } from "./types";

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
