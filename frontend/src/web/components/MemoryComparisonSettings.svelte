<script lang="ts">
  import { onMount } from "svelte";
  import { loadComparisonSettings, saveComparisonSettings } from "../lib/api";
  import type { ComparisonState, ProviderProfile } from "../lib/types";
  let state = $state<ComparisonState | null>(null);
  let providers = $state<ProviderProfile[]>([]);
  let primary = $state("");
  let primaryModel = $state("");
  let fallback = $state("");
  let fallbackModel = $state("");
  let busy = $state(false);
  let error = $state("");
  let saved = $state(false);
  const inputClass = "min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary";
  onMount(() => {
    let active = true;
    void loadComparisonSettings().then(result => {
      if (!active) return;
      state = result.comparison; providers = result.providers;
      primary = state.settings.primary?.provider ?? ""; primaryModel = state.settings.primary?.model ?? "";
      fallback = state.settings.fallback?.provider ?? ""; fallbackModel = state.settings.fallback?.model ?? "";
    }).catch(reason => { if (active) error = reason instanceof Error ? reason.message : "Could not load memory comparison settings."; });
    return () => { active = false; };
  });
  async function save() {
    if (!state) return;
    busy = true; error = ""; saved = false;
    try {
      state = await saveComparisonSettings({ ...$state.snapshot(state.settings), primary: primary ? { provider: primary, model: primaryModel } : null, fallback: primary && fallback ? { provider: fallback, model: fallbackModel } : null });
      saved = true;
    } catch (reason) { error = reason instanceof Error ? reason.message : "Could not save memory comparison settings."; }
    finally { busy = false; }
  }
</script>

<section class="mt-10 border-t border-default pt-6" aria-labelledby="memory-comparison-title">
  <h3 id="memory-comparison-title" class="text-heading">Memory comparison</h3>
  <p class="mt-2 max-w-2xl text-body text-secondary">Compare two related memories before combining them. Uncertain or conflicting memories are kept. Only the selected pair and its context are sent to the providers you choose. A backup is used only if the main provider fails.</p>
  <p class="mt-2 max-w-2xl text-body-sm text-secondary">Jev uses the saved TypeSafe key from Prompt relevance. Other choices use your saved provider credentials. These assignments are independent of Dream extraction.</p>
  {#if state}
    <form class="mt-4 grid gap-4 sm:grid-cols-2" oninput={() => { saved = false; }} onsubmit={event => { event.preventDefault(); void save(); }}>
      <label class="grid gap-1.5 text-caption text-secondary">Primary comparison provider
        <select class={inputClass} bind:value={primary} disabled={busy}>
          <option value="">No model comparison</option><option value="jev">Jev (TypeSafe)</option>
          {#each providers as provider (provider.id)}<option value={provider.id}>{provider.name}</option>{/each}
        </select>
      </label>
      <label class="grid gap-1.5 text-caption text-secondary">Primary comparison model<input class={inputClass} bind:value={primaryModel} required={!!primary} disabled={busy || !primary} /></label>
      <label class="grid gap-1.5 text-caption text-secondary">Backup comparison provider
        <select class={inputClass} bind:value={fallback} disabled={busy || !primary}>
          <option value="">No backup</option><option value="jev">Jev (TypeSafe)</option>
          {#each providers as provider (provider.id)}<option value={provider.id}>{provider.name}</option>{/each}
        </select>
      </label>
      <label class="grid gap-1.5 text-caption text-secondary">Backup comparison model<input class={inputClass} bind:value={fallbackModel} required={!!primary && !!fallback} disabled={busy || !primary || !fallback} /></label>
      <label class="grid gap-1.5 text-caption text-secondary">Pairs per run<input class={inputClass} type="number" min="1" max="32" required bind:value={state.settings.max_pairs_per_run} disabled={busy} /></label>
      <label class="grid gap-1.5 text-caption text-secondary">Request timeout (seconds)<input class={inputClass} type="number" min="1" max="30" required bind:value={state.settings.timeout_seconds} disabled={busy} /></label>
      <p class="sm:col-span-2 text-body-sm text-secondary">Saved primary: {state.primary_ready ? "configured" : "unavailable or disabled"}. Saved backup: {state.fallback_ready ? "configured" : "unavailable or disabled"}. Configuration status does not verify model accuracy.</p>
      <button class="min-h-11 rounded-sm border border-accent bg-raised px-4 py-2 text-body-sm text-accent-text disabled:opacity-60" disabled={busy}>Save memory comparison</button>
    </form>
  {:else if !error}<p class="mt-4 text-body-sm text-secondary">Loading memory comparison settings…</p>{/if}
  {#if error}<p role="alert" class="mt-4 text-body-sm text-danger">{error}</p>{/if}
  {#if saved}<p role="status" class="mt-4 text-body-sm text-secondary">Memory comparison settings saved.</p>{/if}
</section>
