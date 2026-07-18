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
  let sectionRefresh: Promise<void> | null = null;
  let refreshQueued = false;
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

  function refreshSection() {
    if (sectionRefresh) {
      refreshQueued = true;
      return sectionRefresh;
    }
    sectionRefresh = (async () => {
      do {
        refreshQueued = false;
        await loadSection();
      } while (refreshQueued);
    })().finally(() => { sectionRefresh = null; });
    return sectionRefresh;
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
    void refreshSection();
    if (section !== "admin" && section !== "memory") return;
    return connectAdminEvents(() => { void refreshSection(); });
  });
</script>

<main class="min-h-dvh bg-root font-sans text-primary">
  <header class="sticky top-0 z-20 border-b border-default bg-surface">
    <div class="mx-auto flex w-full max-w-[90rem] items-center gap-4 px-4 py-3 sm:px-8 lg:px-12">
    <a class="font-serif text-xl text-primary no-underline" href="/admin">Hieronymus</a>
    <nav class="flex flex-1 items-center gap-1" aria-label="Primary navigation">
      <a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary {section === 'admin' ? 'bg-raised text-primary' : ''}" href="/admin">Overview</a>
      <a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary {section === 'memory' ? 'bg-raised text-primary' : ''}" href="/admin/memory">Memory</a>
      <a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary {!(section === 'admin' || section === 'memory') ? 'bg-raised text-primary' : ''}" href="/config">Config</a>
    </nav>
    <button class="inline-flex min-h-11 shrink-0 items-center gap-2 rounded-sm border border-default px-3 py-2 text-body-sm text-secondary hover:border-accent hover:text-primary" aria-label={themeToggle.theme === "dark" ? "Switch to light theme" : "Switch to dark theme"} onclick={themeToggle.toggle}>
        {#if themeToggle.theme === "dark"}
          <svg aria-hidden="true" viewBox="0 0 20 20" fill="currentColor"><path d="M10 2a1 1 0 0 1 1 1v1a1 1 0 1 1-2 0V3a1 1 0 0 1 1-1Zm0 4a4 4 0 1 0 0 8 4 4 0 0 0 0-8Zm0 10a1 1 0 0 1 1 1v1a1 1 0 1 1-2 0v-1a1 1 0 0 1 1-1Zm-6.36-1.05a1 1 0 0 1 1.41 0l.71.71a1 1 0 0 1-1.42 1.41l-.7-.7a1 1 0 0 1 0-1.42Zm10.61 0a1 1 0 0 1 1.42 1.42l-.71.7a1 1 0 0 1-1.41-1.41l.7-.71ZM3 9h1a1 1 0 1 1 0 2H3a1 1 0 1 1 0-2Zm13 0h1a1 1 0 1 1 0 2h-1a1 1 0 1 1 0-2ZM4.34 3.64a1 1 0 0 1 1.41 0l.71.7a1 1 0 1 1-1.42 1.42l-.7-.71a1 1 0 0 1 0-1.41Zm10.61 0a1 1 0 0 1 0 1.41l-.7.71a1 1 0 1 1-1.42-1.42l.71-.7a1 1 0 0 1 1.41 0Z" /></svg>
        {:else}
          <svg aria-hidden="true" viewBox="0 0 20 20" fill="currentColor"><path d="M17.29 13.29A8 8 0 0 1 6.71 2.71a8 8 0 1 0 10.58 10.58Z" /></svg>
        {/if}
        {themeToggle.theme === "dark" ? "Light" : "Dark"}
    </button>
    </div>
  </header>
  <section class="mx-auto w-full max-w-[90rem] px-4 py-6 sm:px-8 lg:px-12">
    {#if section === "admin" && adminDashboard}
      <AdminDashboard dashboard={adminDashboard} {error} onDream={runDreaming} />
    {:else if section === "memory" && adminDashboard}
      <MemoryViews dashboard={adminDashboard} onNotice={({ message, tone }) => showNotice(message, tone)} />
    {:else if section === "providers"}
      <div class="grid gap-8 md:grid-cols-[minmax(14rem,18rem)_minmax(0,1fr)]">
        <aside class="border-b border-default pb-4 md:border-r md:border-b-0 md:pr-8">
          <p class="mb-4 text-eyebrow uppercase tracking-[0.16em] text-tertiary">Configuration</p>
          <nav class="grid gap-1" aria-label="Configuration sections"><a class="rounded-sm bg-raised px-3 py-2 text-body-sm text-primary no-underline" href="/config">Providers</a><a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary" href="/config/dreaming">Dreaming</a><a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary" href="/config/ingest">Ingest</a><a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary" href="/config/release">Release</a></nav>
        </aside>
        <div class="min-w-0">
        <header class="mb-8 flex flex-col gap-4 border-b border-default pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div><p class="mb-2 text-eyebrow uppercase tracking-[0.16em] text-tertiary">Model access</p><h2 class="text-display">Providers</h2><p class="mt-2 max-w-2xl text-body text-secondary">Manage model-provider profiles for hosted and local models.</p></div>
          <button class="min-h-11 rounded-sm bg-accent px-4 py-2 text-body-sm font-medium text-root hover:opacity-90" onclick={() => { createOpen = true; selected = null; models = []; }}>New provider</button>
        </header>
        {#if error}<p class="mb-5 border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>{/if}
        {#if busy && providers.length === 0}
          <p class="border border-default bg-surface px-4 py-8 text-body text-secondary">Loading profiles…</p>
        {:else if providers.length === 0}
          <div class="overflow-auto border border-default bg-surface"><table class="w-full border-collapse"><tbody><tr><td class="px-4 py-12 text-center text-body text-secondary">No provider profiles yet. Create one to connect an LLM.</td></tr></tbody></table></div>
        {:else}
          <div class="overflow-auto border border-default"><table class="w-full min-w-[42rem] border-collapse text-left"><thead class="bg-surface"><tr><th class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary">Display name</th><th class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary">Type</th><th class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary">Endpoint</th><th class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary">Key</th></tr></thead><tbody>{#each providers as provider (provider.id)}<tr class="cursor-pointer border-b border-default last:border-b-0 hover:bg-raised {selected?.id === provider.id ? 'bg-raised' : ''}" role="button" tabindex="0" onclick={() => { selected = provider; createOpen = false; models = []; }} onkeydown={(event) => { if (event.key === " ") event.preventDefault(); if (event.key === "Enter" || event.key === " ") { selected = provider; createOpen = false; models = []; } }}><td class="px-4 py-3 text-body">{provider.name}</td><td class="px-4 py-3 text-body-sm text-secondary">{provider.type}</td><td class="px-4 py-3 text-body-sm text-secondary">{provider.url}</td><td class="px-4 py-3 text-body-sm text-secondary">{provider.key_configured ? "Configured" : "Missing"}</td></tr>{/each}</tbody></table></div>
        {/if}
        </div>
      </div>
    {:else if section === "dreaming" && dreamSettings}
      <nav class="mb-6 flex flex-wrap gap-1 border-b border-default pb-3" aria-label="Configuration sections"><a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary" href="/config">Providers</a><a class="rounded-sm bg-raised px-3 py-2 text-body-sm text-primary no-underline" href="/config/dreaming">Dreaming</a><a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary" href="/config/ingest">Ingest</a><a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary" href="/config/release">Release</a></nav>{#key "dreaming"}<DreamingEditor initial={dreamSettings} providers={dreamProviders} {modelCache} {busy} {error} onSave={saveDream} />{/key}
    {:else if section === "ingest" && ingestSettings}
      <nav class="mb-6 flex flex-wrap gap-1 border-b border-default pb-3" aria-label="Configuration sections">
        <a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary" href="/config">Providers</a>
        <a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary" href="/config/dreaming">Dreaming</a>
        <a class="rounded-sm bg-raised px-3 py-2 text-body-sm text-primary no-underline" href="/config/ingest">Ingest</a>
        <a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary" href="/config/release">Release</a>
      </nav>
      {#key "ingest"}<IngestEditor initial={ingestSettings} {busy} {error} onSave={saveIngest} />{/key}
    {:else if section === "release" && releaseSettings}
      <nav class="mb-6 flex flex-wrap gap-1 border-b border-default pb-3" aria-label="Configuration sections">
        <a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary" href="/config">Providers</a>
        <a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary" href="/config/dreaming">Dreaming</a>
        <a class="rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary" href="/config/ingest">Ingest</a>
        <a class="rounded-sm bg-raised px-3 py-2 text-body-sm text-primary no-underline" href="/config/release">Release</a>
      </nav>
      {#key "release"}<ReleaseEditor initial={releaseSettings} {busy} {error} onSave={saveRelease} />{/key}
    {:else if error}<p class="border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>
    {:else}<p class="border border-default bg-surface px-4 py-8 text-body text-secondary">Loading settings…</p>{/if}
  </section>
  {#if section === "providers" && (selected || createOpen)}{#key selected?.id ?? "new"}<ProviderEditor provider={selected} {models} {busy} {error} onSave={save} onDelete={remove} onCheck={check} onRefreshModels={refresh} onClose={() => { selected = null; createOpen = false; error = ""; }} />{/key}{/if}
  {#if notice}<Toast message={notice.message} tone={notice.tone} onDismiss={() => { notice = null; }} />{/if}
</main>
