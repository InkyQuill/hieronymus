<script lang="ts">
  import { onMount } from "svelte";
  import { loadRelevanceSettings, saveRelevanceSettings } from "../lib/api";
  import type { RelevanceSettings } from "../lib/types";

  let settings = $state<RelevanceSettings | null>(null);
  let apiKey = $state("");
  let clearKey = $state(false);
  let busy = $state(false);
  let error = $state("");
  let saved = $state(false);
  onMount(() => {
    void loadRelevanceSettings().then(value => { settings = value; }).catch(reason => {
      error = reason instanceof Error ? reason.message : "Could not load relevance settings.";
    });
  });
  async function save() {
    if (!settings) return;
    busy = true; error = ""; saved = false;
    try {
      const { key_configured: _presence, ...values } = $state.snapshot(settings);
      settings = await saveRelevanceSettings({ ...values, api_key: apiKey, clear_key: clearKey });
      apiKey = ""; clearKey = false; saved = true;
    } catch (reason) {
      error = reason instanceof Error ? reason.message : "Could not save relevance settings.";
    } finally { busy = false; }
  }
</script>

<section class="mt-10 border-t border-default pt-6" aria-labelledby="hook-relevance-title">
  <h3 id="hook-relevance-title" class="text-heading">Prompt relevance</h3>
  <p class="mt-2 max-w-2xl text-body text-secondary">With a TypeSafe API key, Jev checks whether a message belongs to your writing project before it enters memory. The current message is sent to TypeSafe for this check. Without a key, or if the service fails, a local word filter is used.</p>
  {#if settings}
    <p class="mt-3 text-body-sm text-secondary">{settings.key_configured ? "Jev API key is saved." : "Local filter is active. No Jev API key is saved."}</p>
    <form class="mt-4 grid gap-4 sm:grid-cols-2" oninput={() => { saved = false; }} onsubmit={event => { event.preventDefault(); void save(); }}>
      <label class="grid gap-1.5 text-caption text-secondary">TypeSafe API key
        <input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary" type="password" autocomplete="new-password" bind:value={apiKey} disabled={busy || clearKey} aria-describedby="jev-key-help" />
      </label>
      <p id="jev-key-help" class="self-center text-body-sm text-secondary">Leave blank to keep the saved key. Removing it restores the local filter.</p>
      <label class="grid gap-1.5 text-caption text-secondary">Jev model
        <input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary" required bind:value={settings.model} disabled={busy} />
      </label>
      <label class="grid gap-1.5 text-caption text-secondary">Timeout (seconds)
        <input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary" type="number" required min="1" max="10" bind:value={settings.timeout_seconds} disabled={busy} />
      </label>
      <label class="grid gap-1.5 text-caption text-secondary">Minimum writing relevance
        <input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary" type="number" required min="0.5" max="1" step="0.01" bind:value={settings.minimum_relevance} disabled={busy} />
      </label>
      <label class="grid gap-1.5 text-caption text-secondary">Maximum technical probability
        <input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary" type="number" required min="0" max="0.5" step="0.01" bind:value={settings.maximum_technical} disabled={busy} />
      </label>
      <label class="flex min-h-11 items-center gap-2 text-body-sm text-secondary"><input type="checkbox" bind:checked={clearKey} disabled={busy || !settings.key_configured} />Remove saved Jev key</label>
      <button class="min-h-11 rounded-sm border border-accent bg-raised px-4 py-2 text-body-sm text-accent-text disabled:opacity-60" disabled={busy}>Save relevance</button>
    </form>
  {:else if !error}<p class="mt-4 text-body-sm text-secondary">Loading relevance settings…</p>{/if}
  {#if error}<p role="alert" class="mt-4 text-body-sm text-danger">{error}</p>{/if}
  {#if saved}<p role="status" class="mt-4 text-body-sm text-secondary">Relevance settings saved.</p>{/if}
</section>
