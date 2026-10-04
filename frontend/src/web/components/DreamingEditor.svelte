<script lang="ts">
  import MemoryComparisonSettings from "./MemoryComparisonSettings.svelte";
  import DreamProviderTimeouts from "./DreamProviderTimeouts.svelte";
  import SettingsSaveState from "./SettingsSaveState.svelte";
  import TechnicalDetails from "./TechnicalDetails.svelte";
  import { workflowGuide } from "../lib/presentation";
  import { refreshModels } from "../lib/api";
  import { onMount } from "svelte";
  import type { DreamSettings, ModelCache, ProviderProfile } from "../lib/types";

  type Props = {
    initial: DreamSettings;
    providers: ProviderProfile[];
    modelCache: ModelCache;
    defaultPrompts?: Record<string, string>;
    busy?: boolean;
    error?: string;
    onSave: (settings: DreamSettings) => void;
  };

  let { initial, providers, modelCache, defaultPrompts = {}, busy = false, error = "", onSave }: Props = $props();
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
  const taskOrder = Object.keys(workflowGuide);
  const orderedTasks = $derived(Object.entries(settings.workflows).sort(([a], [b]) => (taskOrder.indexOf(a) < 0 ? taskOrder.length : taskOrder.indexOf(a)) - (taskOrder.indexOf(b) < 0 ? taskOrder.length : taskOrder.indexOf(b))));

  let availableModels = $state<Record<string, string[]>>({});
  let loadingModels = $state<Record<string, boolean>>({});
  let modelErrors = $state<Record<string, string>>({});
  let customModels = $state<Record<string, boolean>>({});

  async function loadModels(providerId: string) {
    if (!providerId || loadingModels[providerId]) return;
    loadingModels[providerId] = true;
    modelErrors[providerId] = "";
    try {
      availableModels[providerId] = await refreshModels(providerId);
    } catch {
      modelErrors[providerId] = "Could not load models. You can enter a custom model or retry.";
    } finally {
      loadingModels[providerId] = false;
    }
  }

  onMount(() => {
    settings = structuredClone(initial);
    for (const id of new Set(Object.values(initial.workflows).map((workflow) => workflow.provider))) void loadModels(id);
  });

  function modelsFor(providerId: string): string[] {
    return availableModels[providerId] ?? modelCache.providers[providerId]?.models ?? [];
  }

  function updateWorkflow(name: string, changes: Partial<DreamSettings["workflows"][string]>) {
    settings.workflows[name] = { ...settings.workflows[name], ...changes };
  }
</script>

<form class="w-full" aria-label="Dreaming settings" onsubmit={(event) => { event.preventDefault(); onSave($state.snapshot(settings)); }}>
  <header class="flex flex-wrap items-start justify-between gap-4 border-b border-default pb-6">
    <div><h1 class="text-display">Dreaming</h1><p class="mt-2 max-w-2xl text-body text-secondary">Dreaming processes recent memories into lasting knowledge: concepts, terms, writing rules and connections. Choose which AI model handles each task.</p></div>
    <button class="min-h-11 rounded-sm border border-accent bg-raised px-4 py-2 text-body-sm font-medium text-accent-text hover:bg-[var(--hiero-accent-bg)] disabled:cursor-not-allowed disabled:opacity-60" disabled={busy} type="submit">Save dreaming</button>
  </header>
  <SettingsSaveState current={settings} saved={initial} {busy} />
  <fieldset disabled={busy} class="min-w-0" aria-label="Editable settings">

  <h2 class="mt-6 text-h3">When to process memories</h2><p class="mt-2 max-w-[70ch] text-body-sm text-secondary">Enable a schedule for automatic processing. You can also start a run from Overview.</p><div class="mt-4 grid gap-4 sm:grid-cols-2">
    <label class="flex min-h-11 cursor-pointer items-center gap-3 text-body text-primary"><input class="peer sr-only" type="checkbox" bind:checked={settings.dreaming.enabled} /><span class="relative h-[22px] w-10 shrink-0 rounded-full border border-strong bg-raised transition peer-checked:border-accent peer-checked:[&>span]:translate-x-[18px] peer-checked:[&>span]:bg-accent peer-focus-visible:ring-2 peer-focus-visible:ring-accent/40"><span class="absolute top-0.5 left-0.5 size-4 rounded-full bg-secondary transition-transform"></span></span>Enable scheduled dreaming</label>
    <label class="grid gap-1.5 text-caption text-secondary">Interval (minutes)<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" oninvalid={(event) => { const details = event.currentTarget.closest("details"); if (details) details.open = true; }} type="number" required min="1" bind:value={settings.dreaming.schedule_interval_minutes} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Minimum pending memories<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" oninvalid={(event) => { const details = event.currentTarget.closest("details"); if (details) details.open = true; }} type="number" required min="1" bind:value={settings.dreaming.min_pending_short_term_memories} /></label>
  </div>
  <details class="mt-6 border-t border-default pt-2"><summary class="min-h-11 cursor-pointer py-3 text-body-sm text-secondary">Advanced processing limits</summary><p class="mb-4 max-w-[70ch] text-body-sm text-secondary">These limits control how much work a run can attempt. Keep the existing values unless you need to adjust processing size.</p><div class="grid gap-4 sm:grid-cols-2">
    <label class="grid gap-1.5 text-caption text-secondary">Maximum pending memories<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" oninvalid={(event) => { const details = event.currentTarget.closest("details"); if (details) details.open = true; }} type="number" required min="1" bind:value={settings.dreaming.max_pending_short_term_memories} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Maximum memories per run<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" oninvalid={(event) => { const details = event.currentTarget.closest("details"); if (details) details.open = true; }} type="number" required min="1" bind:value={settings.dreaming.max_short_term_memories_per_run} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Working-memory batch size<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" oninvalid={(event) => { const details = event.currentTarget.closest("details"); if (details) details.open = true; }} type="number" required min="1" bind:value={settings.dreaming.max_long_term_records_affected_per_run} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Relation comparison batch size<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" oninvalid={(event) => { const details = event.currentTarget.closest("details"); if (details) details.open = true; }} type="number" required min="1" bind:value={settings.dreaming.max_relation_records_per_pass} /></label>
  </div>

  </details>
  <section class="mt-8 border-t border-default pt-6" aria-labelledby="shared-instructions-heading">
    <h3 id="shared-instructions-heading" class="text-h3">Shared instructions</h3>
    <p id="shared-instructions-help" class="mt-2 max-w-[70ch] text-body-sm text-secondary">Applied to every dreaming pass, alongside its task prompt. Use this for memory language, writing style and project-wide guidance.</p>
    <label class="mt-4 grid gap-1.5 text-caption text-secondary">General prompt<textarea class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" aria-describedby="shared-instructions-help" bind:value={settings.dreaming.general_prompt} rows="4"></textarea></label>
  </section>

  <section class="mt-8"><h2 class="text-h3">Processing tasks</h2><p class="mt-2 max-w-[70ch] text-body-sm text-secondary">Each task uses its own AI connection and model. Enable the tasks you want Hieronymus to perform.</p>{#if providers.length === 0}<p class="mt-3 text-body-sm text-secondary">Add an <a class="text-accent-text underline" href="/config">AI provider</a> before assigning models.</p>{/if}
    {#each orderedTasks as [name, workflow] (name)}
      <article class="mt-6 border-t border-default pt-4"><div>
        <header class="flex items-center justify-between gap-4"><h4 class="text-body font-medium">{workflowGuide[name]?.label ?? name.replaceAll("_", " ")}</h4><label class="flex min-h-11 cursor-pointer items-center"><input class="peer sr-only" type="checkbox" checked={workflow.enabled} onchange={(event) => updateWorkflow(name, { enabled: event.currentTarget.checked })} /><span class="relative h-[22px] w-10 shrink-0 rounded-full border border-strong bg-raised transition peer-checked:border-accent peer-checked:[&>span]:translate-x-[18px] peer-checked:[&>span]:bg-accent peer-focus-visible:ring-2 peer-focus-visible:ring-accent/40"><span class="absolute top-0.5 left-0.5 size-4 rounded-full bg-secondary transition-transform"></span></span><span class="sr-only">Enable {name.replaceAll("_", " ")}</span></label></header>
        <p class="mt-2 max-w-[70ch] text-body-sm text-secondary">{workflowGuide[name]?.description ?? "Configure the model for this task."}</p>
        <details class="mt-3"><summary class="min-h-11 cursor-pointer py-3 text-body-sm text-secondary">Model and task settings · {workflow.model || "Choose a model"}</summary>
        <div class="mt-4 grid items-start gap-4 sm:grid-cols-3">
          <label class="grid gap-1.5 text-caption text-secondary">Provider<select class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" value={workflow.provider} onchange={(event) => { const id = event.currentTarget.value; updateWorkflow(name, { provider: id, model: "" }); customModels[name] = false; void loadModels(id); }}><option value="">Choose profile</option>{#each providers as provider (provider.id)}<option value={provider.id}>{provider.name} · {provider.type}</option>{/each}</select></label>
          <div class="grid gap-1.5 text-caption text-secondary">
            <label class="grid gap-1.5">Model
              <select class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" value={customModels[name] || (workflow.model && !modelsFor(workflow.provider).includes(workflow.model)) ? "__custom__" : workflow.model} onchange={(event) => { customModels[name] = event.currentTarget.value === "__custom__"; if (!customModels[name]) updateWorkflow(name, { model: event.currentTarget.value }); }}>
                <option value="">Choose model</option>
                {#each modelsFor(workflow.provider) as model (model)}<option value={model}>{model}</option>{/each}
                <option value="__custom__">Custom model…</option>
              </select>
            </label>
            {#if customModels[name] || (workflow.model && !modelsFor(workflow.provider).includes(workflow.model))}
              <label class="grid gap-1.5">Custom model<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" value={workflow.model} placeholder="Model ID" oninput={(event) => updateWorkflow(name, { model: event.currentTarget.value })} /></label>
            {/if}
            {#if loadingModels[workflow.provider]}<p role="status">Loading models…</p>{/if}
            {#if modelErrors[workflow.provider]}<p role="status">{modelErrors[workflow.provider]}</p>{/if}
            <button type="button" class="min-h-11 text-left text-accent-text disabled:opacity-60" disabled={!workflow.provider || loadingModels[workflow.provider]} onclick={() => loadModels(workflow.provider)}>Refresh models</button>
          </div>
          <label class="grid gap-1.5 text-caption text-secondary">Output organization guide (records)<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" oninvalid={(event) => { const details = event.currentTarget.closest("details"); if (details) details.open = true; }} type="number" required min="1" bind:value={workflow.max_records_per_pass} /></label>
        </div>
        <details class="mt-5 border-t border-default pt-3">
          <summary class="min-h-11 cursor-pointer py-3 text-body-sm text-accent-text focus-visible:outline-2 focus-visible:outline-accent">Task prompt · {workflow.prompt?.trim() ? "Custom" : "Default"}</summary>
          <p class="mb-3 max-w-[70ch] text-body-sm text-secondary">These instructions apply only to this pass. Shared instructions, source memories and the required JSON format are added automatically.</p>
          <label class="grid gap-1.5 text-caption text-secondary">{name.replaceAll("_", " ")} task prompt<textarea class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" rows="5" value={workflow.prompt?.trim() ? workflow.prompt : (defaultPrompts[name] ?? "")} oninput={(event) => updateWorkflow(name, { prompt: event.currentTarget.value })}></textarea></label>
          <button type="button" class="mt-2 min-h-11 text-body-sm text-accent-text disabled:opacity-60" disabled={!workflow.prompt?.trim()} onclick={() => updateWorkflow(name, { prompt: "" })}>Restore default prompt</button>
        </details>
        </details>
      </div></article>
    {/each}
  </section>
  <footer class="mt-8 flex justify-end border-t border-default pt-4">
    <button class="min-h-11 rounded-sm border border-accent bg-raised px-4 py-2 text-body-sm font-medium text-accent-text hover:bg-[var(--hiero-accent-bg)] disabled:cursor-not-allowed disabled:opacity-60" disabled={busy} type="submit">Save changes</button>
  </footer>
  <TechnicalDetails data={settings} label="Technical Dreaming configuration" />
  {#if error}<p role="alert" class="mt-4 border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>{/if}
  </fieldset>
</form>

<DreamProviderTimeouts {providers} providerIds={providers.map(provider => provider.id)} />
<MemoryComparisonSettings />
