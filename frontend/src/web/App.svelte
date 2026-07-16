<script lang="ts">
  import { onMount } from "svelte";
  import AdminDashboard from "./components/AdminDashboard.svelte";
  import DreamingEditor from "./components/DreamingEditor.svelte";
  import IngestEditor from "./components/IngestEditor.svelte";
  import ProviderEditor from "./components/ProviderEditor.svelte";
  import ReleaseEditor from "./components/ReleaseEditor.svelte";
  import {
    deleteProvider,
    listProviders,
    loadAdminDashboard,
    loadDreamSettings,
    loadIngestSettings,
    loadReleaseSettings,
    refreshModels,
    saveDreamSettings,
    saveIngestSettings,
    saveProvider,
    saveReleaseSettings,
  } from "./lib/api";
  import type {
    AdminDashboard as AdminDashboardPayload,
    DreamSettings,
    IngestSettings,
    ModelCache,
    ProviderDraft,
    ProviderProfile,
    ReleaseSettings,
  } from "./lib/types";

  const path = window.location.pathname;
  const section = path.startsWith("/admin")
    ? "admin"
    : path.endsWith("/dreaming")
    ? "dreaming"
    : path.endsWith("/ingest")
      ? "ingest"
      : path.endsWith("/release")
        ? "release"
        : "providers";
  let providers = $state.raw<ProviderProfile[]>([]);
  let selected = $state.raw<ProviderProfile | null>(null);
  let createOpen = $state(false);
  let models = $state.raw<string[]>([]);
  let busy = $state(false);
  let error = $state("");
  let dreamSettings = $state.raw<DreamSettings | null>(null);
  let dreamProviders = $state.raw<ProviderProfile[]>([]);
  let modelCache = $state.raw<ModelCache>({ providers: {} });
  let ingestSettings = $state.raw<IngestSettings | null>(null);
  let releaseSettings = $state.raw<ReleaseSettings | null>(null);
  let adminDashboard = $state.raw<AdminDashboardPayload | null>(null);

  async function loadProviders() {
    busy = true; error = "";
    try { providers = await listProviders(); }
    catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }

  async function save(draft: ProviderDraft) {
    busy = true; error = "";
    try {
      const saved = await saveProvider(draft);
      await loadProviders();
      selected = saved;
      createOpen = false;
      try {
        models = await refreshModels(saved.id);
      } catch {
        models = [];
      }
    } catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }

  async function refresh() {
    if (!selected) return;
    busy = true; error = "";
    try { models = await refreshModels(selected.id); }
    catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }

  async function remove() {
    if (!selected || !confirm(`Delete ${selected.name}?`)) return;
    busy = true; error = "";
    try { await deleteProvider(selected.id); selected = null; models = []; await loadProviders(); }
    catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }

  async function loadSection() {
    busy = true;
    error = "";
    try {
      if (section === "providers") await loadProviders();
      if (section === "dreaming") {
        const payload = await loadDreamSettings();
        dreamSettings = payload.dream;
        dreamProviders = payload.providers;
        modelCache = payload.model_cache;
      }
      if (section === "ingest") ingestSettings = await loadIngestSettings();
      if (section === "release") releaseSettings = await loadReleaseSettings();
      if (section === "admin") adminDashboard = await loadAdminDashboard();
    } catch (reason) {
      error = reason instanceof Error ? reason.message : String(reason);
    } finally {
      busy = false;
    }
  }

  async function saveDream(settings: DreamSettings) {
    busy = true;
    error = "";
    try { dreamSettings = await saveDreamSettings(settings); }
    catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }

  async function saveIngest(settings: IngestSettings) {
    busy = true;
    error = "";
    try { ingestSettings = await saveIngestSettings(settings); }
    catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }

  async function saveRelease(settings: ReleaseSettings) {
    busy = true;
    error = "";
    try { releaseSettings = await saveReleaseSettings(settings); }
    catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }

  onMount(loadSection);
</script>

<main>
  <aside class="sidebar"><h1>Hieronymus</h1><p>{section === "admin" ? "local administration" : "local configuration"}</p><nav><a class:active={section === "admin"} href="/admin">Overview</a><a class:active={section === "providers"} href="/config">Providers</a><a class:active={section === "dreaming"} href="/config/dreaming">Dreaming</a><a class:active={section === "ingest"} href="/config/ingest">Ingest</a><a class:active={section === "release"} href="/config/release">Release</a></nav><footer>All data is local.<br />No cloud. No tracking.</footer></aside>
  <section class="content">
    {#if section === "admin" && adminDashboard}
      <AdminDashboard dashboard={adminDashboard} {error} />
    {:else if section === "providers"}
      <header class="page-header"><div><h2>Providers</h2><p>Manage custom model-provider profiles.</p></div><button class="primary" onclick={() => { createOpen = true; selected = null; models = []; }}>New provider</button></header>
      {#if error}<p class="error">{error}</p>{/if}
      {#if busy && providers.length === 0}<p>Loading profiles…</p>{:else if providers.length === 0}<div class="empty"><h3>No provider profiles yet</h3><p>Create a profile for OpenAI, DeepSeek, Z.ai, or any compatible endpoint.</p></div>{:else}<table><thead><tr><th>Display name</th><th>Type</th><th>Endpoint</th><th>Key</th></tr></thead><tbody>{#each providers as provider (provider.id)}<tr class:selected={selected?.id === provider.id} onclick={() => { selected = provider; createOpen = false; models = []; }}><td>{provider.name}</td><td>{provider.type}</td><td>{provider.url}</td><td>{provider.key_configured ? "Configured" : "Missing"}</td></tr>{/each}</tbody></table>{/if}
    {:else if section === "dreaming" && dreamSettings}
      {#key "dreaming"}<DreamingEditor initial={dreamSettings} providers={dreamProviders} {modelCache} {busy} {error} onSave={saveDream} />{/key}
    {:else if section === "ingest" && ingestSettings}
      {#key "ingest"}<IngestEditor initial={ingestSettings} {busy} {error} onSave={saveIngest} />{/key}
    {:else if section === "release" && releaseSettings}
      {#key "release"}<ReleaseEditor initial={releaseSettings} {busy} {error} onSave={saveRelease} />{/key}
    {:else if error}<p class="error">{error}</p>
    {:else}<p>Loading settings…</p>{/if}
  </section>
  {#if section === "providers" && (selected || createOpen)}{#key selected?.id ?? "new"}<ProviderEditor provider={selected} {models} {busy} {error} onSave={save} onDelete={remove} onRefreshModels={refresh} onClose={() => { selected = null; createOpen = false; error = ""; }} />{/key}{/if}
</main>
