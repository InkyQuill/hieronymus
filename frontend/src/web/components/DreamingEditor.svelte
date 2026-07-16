<script lang="ts">
  import { onMount } from "svelte";
  import type { DreamSettings, ModelCache, ProviderProfile } from "../lib/types";

  type Props = {
    initial: DreamSettings;
    providers: ProviderProfile[];
    modelCache: ModelCache;
    busy?: boolean;
    error?: string;
    onSave: (settings: DreamSettings) => void;
  };

  let { initial, providers, modelCache, busy = false, error = "", onSave }: Props = $props();
  const emptySettings = (): DreamSettings => ({
    dreaming: {
      enabled: false,
      schedule_interval_minutes: 30,
      min_pending_short_term_memories: 20,
      max_pending_short_term_memories: 200,
      max_short_term_memories_per_cycle: 50,
      not_enough_memories_cycle_threshold: 5,
      max_changed_crystals_per_cycle: 200,
      max_related_concepts_per_cycle: 80,
      max_related_crystals_per_concept: 20,
      max_total_affected_crystals: 500,
      general_prompt: "",
    },
    workflows: {},
  });
  let settings = $state<DreamSettings>(emptySettings());

  onMount(() => {
    settings = structuredClone(initial);
  });

  function modelsFor(providerId: string): string[] {
    return modelCache.providers[providerId]?.models ?? [];
  }

  function updateWorkflow(name: string, changes: Partial<DreamSettings["workflows"][string]>) {
    settings.workflows[name] = { ...settings.workflows[name], ...changes };
  }
</script>

<section class="settings" aria-label="Dreaming settings">
  <header class="page-header">
    <div><h2>Dreaming</h2><p>Schedule and model workflows for local memory maintenance.</p></div>
    <button class="primary" disabled={busy} onclick={() => onSave($state.snapshot(settings))}>Save dreaming</button>
  </header>

  <div class="settings-grid">
    <label class="toggle"><input type="checkbox" bind:checked={settings.dreaming.enabled} /> Enable scheduled dreaming</label>
    <label>Interval (minutes)<input type="number" min="1" bind:value={settings.dreaming.schedule_interval_minutes} /></label>
    <label>Minimum pending memories<input type="number" min="1" bind:value={settings.dreaming.min_pending_short_term_memories} /></label>
    <label>Maximum pending memories<input type="number" min="1" bind:value={settings.dreaming.max_pending_short_term_memories} /></label>
    <label>Maximum memories per cycle<input type="number" min="1" bind:value={settings.dreaming.max_short_term_memories_per_cycle} /></label>
  </div>

  <section class="workflows"><h3>Workflows</h3>
    {#each Object.entries(settings.workflows) as [name, workflow] (name)}
      <article class="workflow">
        <header><div><h4>{name.replaceAll("_", " ")}</h4><label class="toggle"><input type="checkbox" checked={workflow.enabled} onchange={(event) => updateWorkflow(name, { enabled: event.currentTarget.checked })} /> Enabled</label></div></header>
        <div class="settings-grid compact">
          <label>Provider<select value={workflow.provider} onchange={(event) => updateWorkflow(name, { provider: event.currentTarget.value, model: "" })}><option value="">Choose profile</option>{#each providers as provider (provider.id)}<option value={provider.id}>{provider.name} · {provider.type}</option>{/each}</select></label>
          <label>Model
            {#if modelsFor(workflow.provider).length}
              <select value={workflow.model} onchange={(event) => updateWorkflow(name, { model: event.currentTarget.value })}><option value="">Choose model</option>{#each modelsFor(workflow.provider) as model (model)}<option value={model}>{model}</option>{/each}</select>
            {:else}
              <input value={workflow.model} placeholder="Model ID" oninput={(event) => updateWorkflow(name, { model: event.currentTarget.value })} />
            {/if}
          </label>
        </div>
      </article>
    {/each}
  </section>
  <label class="prompt">General prompt<textarea bind:value={settings.dreaming.general_prompt} rows="5"></textarea></label>
  {#if error}<p class="error">{error}</p>{/if}
</section>
