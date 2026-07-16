<script lang="ts">
  import { onMount } from "svelte";
  import ProviderEditor from "./components/ProviderEditor.svelte";
  import { deleteProvider, listProviders, refreshModels, saveProvider } from "./lib/api";
  import type { ProviderDraft, ProviderProfile } from "./lib/types";

  const path = window.location.pathname;
  let providers = $state.raw<ProviderProfile[]>([]);
  let selected = $state.raw<ProviderProfile | null>(null);
  let createOpen = $state(false);
  let models = $state.raw<string[]>([]);
  let busy = $state(false);
  let error = $state("");

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

  onMount(loadProviders);
</script>

<main>
  <aside class="sidebar"><h1>Hieronymus</h1><p>local configuration</p><nav><a href="/admin">Overview</a><a class:active={path.startsWith("/config")} href="/config">Providers</a><a href="/config/dreaming">Dreaming</a><a href="/config/ingest">Ingest</a><a href="/config/release">Release</a></nav><footer>All data is local.<br />No cloud. No tracking.</footer></aside>
  <section class="content"><header class="page-header"><div><h2>Providers</h2><p>Manage custom model-provider profiles.</p></div><button class="primary" onclick={() => { createOpen = true; selected = null; models = []; }}>New provider</button></header>
    {#if error}<p class="error">{error}</p>{/if}
    {#if busy && providers.length === 0}<p>Loading profiles…</p>{:else if providers.length === 0}<div class="empty"><h3>No provider profiles yet</h3><p>Create a profile for OpenAI, DeepSeek, Z.ai, or any compatible endpoint.</p></div>{:else}<table><thead><tr><th>Display name</th><th>Type</th><th>Endpoint</th><th>Key</th></tr></thead><tbody>{#each providers as provider (provider.id)}<tr class:selected={selected?.id === provider.id} onclick={() => { selected = provider; createOpen = false; models = []; }}><td>{provider.name}</td><td>{provider.type}</td><td>{provider.url}</td><td>{provider.key_configured ? "Configured" : "Missing"}</td></tr>{/each}</tbody></table>{/if}
  </section>
  {#if selected || createOpen}{#key selected?.id ?? "new"}<ProviderEditor provider={selected} {models} {busy} {error} onSave={save} onDelete={remove} onRefreshModels={refresh} onClose={() => { selected = null; createOpen = false; error = ""; }} />{/key}{/if}
</main>
