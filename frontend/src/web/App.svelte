<script lang="ts">
  import Sun from "@lucide/svelte/icons/sun";
  import Moon from "@lucide/svelte/icons/moon";
  import SettingsNavigation from "./components/SettingsNavigation.svelte";
  import { onMount } from "svelte";
  import { connectAdminEvents } from "./lib/admin-events.svelte";
  import ConnectAgent from "./components/ConnectAgent.svelte";
  import AdminDashboard from "./components/AdminDashboard.svelte";
  import MemoryViews from "./components/MemoryViews.svelte";
  import DreamingEditor from "./components/DreamingEditor.svelte";
  import IngestEditor from "./components/IngestEditor.svelte";
  import ProviderEditor from "./components/ProviderEditor.svelte";
  import ReleaseEditor from "./components/ReleaseEditor.svelte";
  import VersionInfo from "./components/VersionInfo.svelte";
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
  const section = path === "/" || path === "/admin/connect"
    ? "connect"
    : path === "/admin/memory"
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
  let bookHeader = $state<HTMLDivElement>();
  let providers = $state.raw<ProviderProfile[]>([]);
  let selected = $state.raw<ProviderProfile | null>(null);
  let createOpen = $state(false);
  let models = $state.raw<string[]>([]);
  let busy = $state(false);
  let error = $state("");
  let dreamSettings = $state.raw<DreamSettings | null>(null);
  let dreamProviders = $state.raw<ProviderProfile[]>([]);
  let defaultPrompts = $state.raw<Record<string, string>>({});
  let modelCache = $state.raw<ModelCache>({ providers: {} });
  let ingestSettings = $state.raw<IngestSettings | null>(null);
  let releaseSettings = $state.raw<ReleaseSettings | null>(null);
  let adminDashboard = $state.raw<AdminDashboardPayload | null>(null);
  let notice = $state.raw<{ message: string; tone: "success" | "error" } | null>(null);
  let sectionRefresh: Promise<void> | null = null;
  let refreshQueued = false;
  let dashboardRequestSequence = 0;
  let dashboardPollError = $state("");
  let dashboardPollPending = false;
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
    if (!selected) return;
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
        defaultPrompts = payload.default_prompts ?? {};
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
    dashboardRequestSequence += 1;
    dashboardPollError = "";
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
    dashboardRequestSequence += 1;
    dashboardPollError = "";
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
    const disconnect = connectAdminEvents(() => { void refreshSection(); });
    // Autonomous indexing can progress without a Dreaming event or a recall.
    const timer = section === "admin" ? setInterval(() => {
      if (!busy && !sectionRefresh && !dashboardPollPending) {
        const sequence = ++dashboardRequestSequence;
        dashboardPollPending = true;
        void loadAdminDashboard().then(value => {
          if (sequence !== dashboardRequestSequence) return;
          adminDashboard = value;
          dashboardPollError = "";
        }).catch(reason => {
          if (sequence === dashboardRequestSequence) dashboardPollError = reason instanceof Error ? reason.message : String(reason);
        }).finally(() => { dashboardPollPending = false; });
      }
    }, 2_000) : undefined;
    return () => { dashboardRequestSequence += 1; disconnect(); if (timer !== undefined) clearInterval(timer); };
  });
</script>

<a href="#page-content" class="skip-link">Skip to page content</a>
<main class="min-h-dvh bg-root font-sans text-primary">
  <header class="z-20 sm:sticky sm:top-0 border-b border-default bg-surface">
    <div class="mx-auto flex w-full flex-wrap items-center justify-between gap-4 px-4 py-3 sm:flex-nowrap sm:px-8 lg:px-12">
    <div><a class="font-serif text-xl text-primary no-underline" href="/admin">Hieronymus</a>
      <VersionInfo />
    </div>
    <nav class="order-last flex w-full min-w-0 flex-wrap items-center gap-1 sm:order-none sm:w-auto sm:flex-1" aria-label="Primary navigation">
      <a class="inline-flex min-h-11 items-center rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary {section === 'connect' ? 'bg-raised text-primary' : ''}" href="/admin/connect" aria-current={section === "connect" ? "page" : undefined}>Connect your agent</a>
      <a class="inline-flex min-h-11 items-center rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary {section === 'admin' ? 'bg-raised text-primary' : ''}" href="/admin" aria-current={section === "admin" ? "page" : undefined}>Overview</a>
      <a class="inline-flex min-h-11 items-center rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary {section === 'memory' ? 'bg-raised text-primary' : ''}" href="/admin/memory" aria-current={section === "memory" ? "page" : undefined}>Memory</a>
      <a class="inline-flex min-h-11 items-center rounded-sm px-3 py-2 text-body-sm text-secondary no-underline hover:bg-raised hover:text-primary {!(section === 'admin' || section === 'memory' || section === 'connect') ? 'bg-raised text-primary' : ''}" href="/config" aria-current={!["admin", "memory", "connect"].includes(section) ? "page" : undefined}>Settings</a>
    </nav>
    <div bind:this={bookHeader}></div>
    <button class="inline-flex min-h-11 shrink-0 items-center gap-2 rounded-sm border border-default px-3 py-2 text-body-sm text-secondary hover:border-accent hover:text-primary" aria-label={themeToggle.theme === "dark" ? "Switch to light theme" : "Switch to dark theme"} onclick={themeToggle.toggle}>
        {#if themeToggle.theme === "dark"}
          <Sun size={20} aria-hidden="true" />
        {:else}
          <Moon size={20} aria-hidden="true" />
        {/if}
        {themeToggle.theme === "dark" ? "Light" : "Dark"}
    </button>
    </div>
  </header>
  <section id="page-content" tabindex="-1" class="mx-auto w-full px-4 py-6 sm:px-8 lg:px-12">
    {#if ["providers", "dreaming", "ingest", "release"].includes(section)}<SettingsNavigation {section} />{/if}
    {#if (error || dashboardPollError) && (section === "providers" || (["admin", "memory"].includes(section) && adminDashboard))}<div role="alert" class="mb-4 border border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger"><p>{error || dashboardPollError}</p><button class="mt-2 min-h-11 rounded-sm border border-danger px-4 py-2" disabled={busy} onclick={() => { void refreshSection(); }}>Try again</button></div>{/if}
    {#if section === "connect"}
      <ConnectAgent />
    {:else if section === "admin" && adminDashboard}
      <AdminDashboard dashboard={adminDashboard} {busy} onDream={runDreaming} />
    {:else if section === "memory" && adminDashboard}
      <MemoryViews dashboard={adminDashboard} {bookHeader} onNotice={({ message, tone }) => showNotice(message, tone)} />
    {:else if section === "providers"}
      <div class="w-full">
        <div class="min-w-0">
        <header class="mb-8 flex flex-col gap-4 border-b border-default pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div><h1 class="text-display">AI providers</h1><p class="mt-2 max-w-2xl text-body text-secondary">Connect the AI services used to process your memories. Your writing agent is connected separately.</p></div>
          <button class="min-h-11 rounded-sm bg-accent px-4 py-2 text-body-sm font-medium text-root hover:opacity-90" onclick={() => { createOpen = true; selected = null; models = []; }}>New provider</button>
        </header>

        {#if busy && providers.length === 0}
          <p class="border border-default bg-surface px-4 py-8 text-body text-secondary">Loading profiles…</p>
        {:else if providers.length === 0 && !error}
          <div class="overflow-auto border border-default bg-surface"><table class="w-full border-collapse"><tbody><tr><td class="px-4 py-12 text-center text-body text-secondary">No AI providers yet. Add a hosted service or local Ollama server to use Dreaming.</td></tr></tbody></table></div>
        {:else if providers.length > 0}
          <div class="overflow-auto border border-default"><table class="w-full min-w-[42rem] border-collapse text-left"><thead class="bg-surface"><tr><th class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary">Display name</th><th class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary">Type</th><th class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary">Endpoint</th><th class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary">Key</th></tr></thead><tbody>{#each providers as provider (provider.id)}<tr class="cursor-pointer border-b border-default last:border-b-0 hover:bg-raised {selected?.id === provider.id ? 'bg-raised' : ''}" ><td class="px-4 py-3 text-body"><button class="min-h-11 w-full text-left text-accent-text underline" onclick={() => { selected = provider; createOpen = false; models = []; }} aria-label={`Edit ${provider.name}`}>{provider.name}</button></td><td class="px-4 py-3 text-body-sm text-secondary">{provider.type}</td><td class="px-4 py-3 text-body-sm text-secondary">{provider.url}</td><td class="px-4 py-3 text-body-sm text-secondary">{provider.key_configured ? "Configured" : provider.type === "ollama" ? "Not configured (optional)" : "Not configured"}</td></tr>{/each}</tbody></table></div>
        {/if}
        <p class="mt-4 text-body-sm text-secondary">Open a provider to edit its connection or check it. Then assign its models in <a href="/config/dreaming" class="text-accent-text underline">Dreaming settings</a>.</p>
        </div>
      </div>
    {:else if section === "dreaming" && dreamSettings}
      {#key "dreaming"}<DreamingEditor initial={dreamSettings} providers={dreamProviders} {defaultPrompts} {modelCache} {busy} {error} onSave={saveDream} />{/key}
    {:else if section === "ingest" && ingestSettings}

      {#key "ingest"}<IngestEditor initial={ingestSettings} {busy} {error} onSave={saveIngest} />{/key}
    {:else if section === "release" && releaseSettings}

      {#key "release"}<ReleaseEditor initial={releaseSettings} {busy} {error} onSave={saveRelease} />{/key}
    {:else if error}<div role="alert" class="border border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger"><p>{error}</p><button class="mt-3 min-h-11 rounded-sm border border-danger px-4 py-2" onclick={() => { void refreshSection(); }}>Try again</button></div>
    {:else}<p class="border border-default bg-surface px-4 py-8 text-body text-secondary">Loading {section === "admin" ? "overview" : section === "memory" ? "memory" : "settings"}…</p>{/if}
  </section>
  {#if section === "providers" && (selected || createOpen)}{#key selected?.id ?? "new"}<ProviderEditor provider={selected} {models} {busy} {error} onSave={save} onDelete={remove} onCheck={check} onRefreshModels={refresh} onClose={() => { selected = null; createOpen = false; error = ""; }} />{/key}{/if}
  {#if notice}<Toast message={notice.message} tone={notice.tone} onDismiss={() => { notice = null; }} />{/if}
</main>
