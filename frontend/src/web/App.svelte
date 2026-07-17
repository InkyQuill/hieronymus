<script lang="ts">
  import { onMount } from "svelte";
  import { connectAdminEvents } from "./lib/admin-events.svelte";
  import AdminDashboard from "./components/AdminDashboard.svelte";
  import MemoryViews from "./components/MemoryViews.svelte";
  import DreamingEditor from "./components/DreamingEditor.svelte";
  import IngestEditor from "./components/IngestEditor.svelte";
  import ProviderEditor from "./components/ProviderEditor.svelte";
  import ReleaseEditor from "./components/ReleaseEditor.svelte";
  import Toast from "./components/Toast.svelte";
  import { createThemeToggle } from "./lib/theme.svelte";
  import {
    deleteProvider,
    checkProvider,
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
    startAdminDreaming,
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
  const section = path === "/admin/memory"
    ? "memory"
    : path.startsWith("/admin")
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
  let notice = $state.raw<{ message: string; tone: "success" | "error" } | null>(null);
  const themeToggle = createThemeToggle();

  function showNotice(message: string, tone: "success" | "error" = "success") {
    notice = { message, tone };
  }

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
      showNotice(`Saved ${saved.name}.`);
    } catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }

  async function refresh() {
    if (!selected) return;
    busy = true; error = "";
    try { models = await refreshModels(selected.id); showNotice("Model list refreshed."); }
    catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }

  async function check() {
    if (!selected) return;
    busy = true;
    error = "";
    try {
      const result = await checkProvider(selected.id);
      models = result.models;
      showNotice(
        result.ok ? "Connection verified." : `Connection check failed: ${result.error}`,
        result.ok ? "success" : "error",
      );
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason);
      error = message;
      showNotice(`Connection check failed: ${message}`, "error");
    } finally { busy = false; }
  }

  async function remove() {
    if (!selected || !confirm(`Delete ${selected.name}?`)) return;
    busy = true; error = "";
    try { await deleteProvider(selected.id); selected = null; models = []; await loadProviders(); showNotice("Provider deleted."); }
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
      if (section === "admin" || section === "memory") adminDashboard = await loadAdminDashboard();
    } catch (reason) {
      error = reason instanceof Error ? reason.message : String(reason);
    } finally {
      busy = false;
    }
  }

  async function saveDream(settings: DreamSettings) {
    busy = true;
    error = "";
    try { dreamSettings = await saveDreamSettings(settings); showNotice("Dreaming settings saved."); }
    catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }

  async function saveIngest(settings: IngestSettings) {
    busy = true;
    error = "";
    try { ingestSettings = await saveIngestSettings(settings); showNotice("Ingest settings saved."); }
    catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }

  async function saveRelease(settings: ReleaseSettings) {
    busy = true;
    error = "";
    try { releaseSettings = await saveReleaseSettings(settings); showNotice("Release settings saved."); }
    catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }

  async function runDreaming() {
    busy = true;
    error = "";
    try {
      const result = await startAdminDreaming();
      showNotice(result.started ? "Dreaming started." : "Dreaming is already running.");
      adminDashboard = await loadAdminDashboard();
    } catch (reason) {
      error = reason instanceof Error ? reason.message : String(reason);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void loadSection();
    return connectAdminEvents(() => {
      if (section === "admin" || section === "memory") void loadSection();
    });
  });
</script>

<main>
  <aside class="sidebar">
    <h1>Hieronymus</h1>
    <p>{section === "admin" || section === "memory" ? "local administration" : "local configuration"}</p>
    <nav aria-label="Primary navigation">
      <a class:active={section === "admin"} href="/admin">Overview</a>
      <a class:active={section === "memory"} href="/admin/memory">Memory views</a>
      <a class:active={section === "providers"} href="/config">Providers</a>
      <a class:active={section === "dreaming"} href="/config/dreaming">Dreaming</a>
      <a class:active={section === "ingest"} href="/config/ingest">Ingest</a>
      <a class:active={section === "release"} href="/config/release">Release</a>
    </nav>
    <footer>
      All data is local.<br />No cloud. No tracking.
      <button class="theme-toggle" aria-label={themeToggle.theme === "dark" ? "Switch to light theme" : "Switch to dark theme"} onclick={themeToggle.toggle}>
        {#if themeToggle.theme === "dark"}
          <svg aria-hidden="true" viewBox="0 0 20 20" fill="currentColor"><path d="M10 2a1 1 0 0 1 1 1v1a1 1 0 1 1-2 0V3a1 1 0 0 1 1-1Zm0 4a4 4 0 1 0 0 8 4 4 0 0 0 0-8Zm0 10a1 1 0 0 1 1 1v1a1 1 0 1 1-2 0v-1a1 1 0 0 1 1-1Zm-6.36-1.05a1 1 0 0 1 1.41 0l.71.71a1 1 0 0 1-1.42 1.41l-.7-.7a1 1 0 0 1 0-1.42Zm10.61 0a1 1 0 0 1 1.42 1.42l-.71.7a1 1 0 0 1-1.41-1.41l.7-.71ZM3 9h1a1 1 0 1 1 0 2H3a1 1 0 1 1 0-2Zm13 0h1a1 1 0 1 1 0 2h-1a1 1 0 1 1 0-2ZM4.34 3.64a1 1 0 0 1 1.41 0l.71.7a1 1 0 1 1-1.42 1.42l-.7-.71a1 1 0 0 1 0-1.41Zm10.61 0a1 1 0 0 1 0 1.41l-.7.71a1 1 0 1 1-1.42-1.42l.71-.7a1 1 0 0 1 1.41 0Z" /></svg>
        {:else}
          <svg aria-hidden="true" viewBox="0 0 20 20" fill="currentColor"><path d="M17.29 13.29A8 8 0 0 1 6.71 2.71a8 8 0 1 0 10.58 10.58Z" /></svg>
        {/if}
        {themeToggle.theme === "dark" ? "Light" : "Dark"}
      </button>
    </footer>
  </aside>
  <section class="workspace">
    {#if section === "admin" && adminDashboard}
      <AdminDashboard dashboard={adminDashboard} {error} onDream={runDreaming} />
    {:else if section === "memory" && adminDashboard}
      <MemoryViews dashboard={adminDashboard} onNotice={({ message, tone }) => showNotice(message, tone)} />
    {:else if section === "providers"}
      <div class="settings settings-page">
        <header class="page-header">
          <div><h2>Providers</h2><p>Manage model-provider profiles for hosted and local models.</p></div>
          <button class="btn-primary" onclick={() => { createOpen = true; selected = null; models = []; }}>New provider</button>
        </header>
        {#if error}<p class="error-msg">{error}</p>{/if}
        {#if busy && providers.length === 0}
          <p class="loading">Loading profiles…</p>
        {:else if providers.length === 0}
          <div class="table-wrap"><table><tbody><tr><td class="empty-cell">No provider profiles yet. Create one to connect an LLM.</td></tr></tbody></table></div>
        {:else}
          <div class="table-wrap"><table><thead><tr><th>Display name</th><th>Type</th><th>Endpoint</th><th>Key</th></tr></thead><tbody>{#each providers as provider (provider.id)}<tr class:selected={selected?.id === provider.id} role="button" tabindex="0" onclick={() => { selected = provider; createOpen = false; models = []; }} onkeydown={(event) => { if (event.key === "Enter" || event.key === " ") { selected = provider; createOpen = false; models = []; } }}><td>{provider.name}</td><td>{provider.type}</td><td>{provider.url}</td><td>{provider.key_configured ? "Configured" : "Missing"}</td></tr>{/each}</tbody></table></div>
        {/if}
      </div>
    {:else if section === "dreaming" && dreamSettings}
      {#key "dreaming"}<DreamingEditor initial={dreamSettings} providers={dreamProviders} {modelCache} {busy} {error} onSave={saveDream} />{/key}
    {:else if section === "ingest" && ingestSettings}
      {#key "ingest"}<IngestEditor initial={ingestSettings} {busy} {error} onSave={saveIngest} />{/key}
    {:else if section === "release" && releaseSettings}
      {#key "release"}<ReleaseEditor initial={releaseSettings} {busy} {error} onSave={saveRelease} />{/key}
    {:else if error}<p class="error-msg">{error}</p>
    {:else}<p class="loading">Loading settings…</p>{/if}
  </section>
  {#if section === "providers" && (selected || createOpen)}{#key selected?.id ?? "new"}<div class="editor-backdrop" onclick={() => { selected = null; createOpen = false; error = ""; }} role="presentation"></div><ProviderEditor provider={selected} {models} {busy} {error} onSave={save} onDelete={remove} onCheck={check} onRefreshModels={refresh} onClose={() => { selected = null; createOpen = false; error = ""; }} />{/key}{/if}
  {#if notice}<Toast message={notice.message} tone={notice.tone} onDismiss={() => { notice = null; }} />{/if}
</main>
