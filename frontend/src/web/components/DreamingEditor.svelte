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
      max_short_term_memories_per_run: 500,
      max_long_term_records_affected_per_run: 1000,
      max_relation_records_per_pass: 1000,
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

<section class="mx-auto max-w-5xl" aria-label="Dreaming settings">
  <header class="flex flex-wrap items-start justify-between gap-4 border-b border-default pb-6">
    <div><h2 class="text-display">Dreaming</h2><p class="mt-2 max-w-2xl text-body text-secondary">Seven evidence-tracked passes turn completed reading memory into durable knowledge.</p></div>
    <button class="min-h-11 rounded-sm border border-accent bg-raised px-4 py-2 text-body-sm font-medium text-accent hover:bg-[var(--hiero-accent-bg)] disabled:cursor-not-allowed disabled:opacity-60" disabled={busy} onclick={() => onSave($state.snapshot(settings))}>Save dreaming</button>
  </header>

  <div class="mt-6 grid gap-4 sm:grid-cols-2">
    <label class="flex min-h-11 cursor-pointer items-center gap-3 text-body text-primary"><input class="peer sr-only" type="checkbox" bind:checked={settings.dreaming.enabled} /><span class="relative h-[22px] w-10 shrink-0 rounded-full border border-strong bg-raised transition peer-checked:border-accent peer-focus-visible:ring-2 peer-focus-visible:ring-accent/40"><span class="absolute top-0.5 left-0.5 size-4 rounded-full bg-secondary transition-transform peer-checked:translate-x-[18px] peer-checked:bg-accent"></span></span>Enable scheduled dreaming</label>
    <label class="grid gap-1.5 text-caption text-secondary">Interval (minutes)<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" type="number" min="1" bind:value={settings.dreaming.schedule_interval_minutes} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Minimum pending memories<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" type="number" min="1" bind:value={settings.dreaming.min_pending_short_term_memories} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Maximum pending memories<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" type="number" min="1" bind:value={settings.dreaming.max_pending_short_term_memories} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Maximum memories per run<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" type="number" min="1" max="500" bind:value={settings.dreaming.max_short_term_memories_per_run} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Maximum long-term records per run<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" type="number" min="1" bind:value={settings.dreaming.max_long_term_records_affected_per_run} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Maximum relations per pass<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" type="number" min="1" bind:value={settings.dreaming.max_relation_records_per_pass} /></label>
  </div>

  <section class="mt-8"><h3 class="text-h3">Workflows</h3>
    {#each Object.entries(settings.workflows) as [name, workflow] (name)}
      <article class="mt-3 rounded-md border border-default bg-surface p-4"><div>
        <header class="flex items-center justify-between gap-4"><h4 class="text-body font-medium capitalize">{name.replaceAll("_", " ")}</h4><label class="flex min-h-11 cursor-pointer items-center"><input class="peer sr-only" type="checkbox" checked={workflow.enabled} onchange={(event) => updateWorkflow(name, { enabled: event.currentTarget.checked })} /><span class="relative h-[22px] w-10 shrink-0 rounded-full border border-strong bg-raised transition peer-checked:border-accent peer-focus-visible:ring-2 peer-focus-visible:ring-accent/40"><span class="absolute top-0.5 left-0.5 size-4 rounded-full bg-secondary transition-transform peer-checked:translate-x-[18px] peer-checked:bg-accent"></span></span><span class="sr-only">Enable {name.replaceAll("_", " ")}</span></label></header>
        <div class="mt-4 grid gap-4 sm:grid-cols-3">
          <label class="grid gap-1.5 text-caption text-secondary">Provider<select class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" value={workflow.provider} onchange={(event) => updateWorkflow(name, { provider: event.currentTarget.value, model: "" })}><option value="">Choose profile</option>{#each providers as provider (provider.id)}<option value={provider.id}>{provider.name} · {provider.type}</option>{/each}</select></label>
          <label class="grid gap-1.5 text-caption text-secondary">Model
            {#if modelsFor(workflow.provider).length}
              <select class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" value={workflow.model} onchange={(event) => updateWorkflow(name, { model: event.currentTarget.value })}><option value="">Choose model</option>{#each modelsFor(workflow.provider) as model (model)}<option value={model}>{model}</option>{/each}</select>
            {:else}
              <input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" value={workflow.model} placeholder="Model ID" oninput={(event) => updateWorkflow(name, { model: event.currentTarget.value })} />
            {/if}
          </label>
          <label class="grid gap-1.5 text-caption text-secondary">Maximum records<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" type="number" min="1" bind:value={workflow.max_records_per_pass} /></label>
        </div>
      </div></article>
    {/each}
  </section>
  <label class="mt-8 grid gap-1.5 text-caption text-secondary">General prompt<textarea class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" bind:value={settings.dreaming.general_prompt} rows="5"></textarea></label>
  {#if error}<p class="mt-4 border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>{/if}
</section>
