<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import type { ProviderDraft, ProviderProfile } from "../lib/types";

  type Props = {
    provider?: ProviderProfile | null;
    models?: string[];
    busy?: boolean;
    error?: string;
    onSave: (draft: ProviderDraft) => void;
    onDelete: () => void;
    onRefreshModels: () => void;
    onCheck: () => void;
    onClose: () => void;
  };

  let { provider = null, models = [], busy = false, error = "", onSave, onDelete, onRefreshModels, onCheck, onClose }: Props = $props();
  const blankDraft = (): ProviderDraft => ({ id: "", name: "", type: "openai", url: "", key: "", timeout_seconds: "30" });
  let draft = $state<ProviderDraft>(blankDraft());
  let dialog: HTMLDialogElement;
  let previouslyFocused: HTMLElement | null = null;

  onMount(() => {
    previouslyFocused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    draft = provider
      ? {
          id: provider.id,
          name: provider.name,
          type: provider.type,
          url: provider.url,
          key: "",
          timeout_seconds: String(provider.timeout_seconds),
        }
      : blankDraft();
    dialog.showModal();
    dialog.querySelector<HTMLElement>("input:not([disabled]), select, button")?.focus();
  });

  onDestroy(() => previouslyFocused?.focus());
  function submit() { onSave(draft); }
</script>

<dialog bind:this={dialog} class="editor-dialog text-primary backdrop:bg-black/30 dark:backdrop:bg-black/60" aria-labelledby="provider-editor-title" oncancel={(event) => { event.preventDefault(); onClose(); }}>
  <header class="flex items-center justify-between gap-4 border-b border-default pb-4"><h2 id="provider-editor-title" class="text-h2">{provider ? `Edit ${provider.name}` : "New provider"}</h2><button class="inline-flex size-11 items-center justify-center rounded-sm border border-default bg-surface text-2xl leading-none text-secondary hover:bg-raised hover:text-primary" aria-label="Close editor" onclick={onClose}>&times;</button></header>
  <form class="mt-5 grid gap-4" onsubmit={(event) => { event.preventDefault(); submit(); }}>
    <label class="grid gap-1.5 text-caption text-secondary">Profile ID<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary disabled:cursor-not-allowed disabled:opacity-60 focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" bind:value={draft.id} disabled={provider !== null} required pattern="[A-Za-z0-9_-]+" /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Display name<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" bind:value={draft.name} required /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Provider type<select class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" bind:value={draft.type}><option value="openai">OpenAI compatible</option><option value="google">Google GenAI</option><option value="anthropic">Anthropic</option><option value="ollama">Ollama</option></select></label>
    <label class="grid gap-1.5 text-caption text-secondary">Endpoint<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" bind:value={draft.url} required placeholder="https://api.example.com/v1" /></label>
    <label class="grid gap-1.5 text-caption text-secondary">API key<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" bind:value={draft.key} type="password" placeholder={provider?.key_configured ? "Stored key (leave blank to keep)" : "Required for remote providers"} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Timeout (seconds)<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" bind:value={draft.timeout_seconds} inputmode="numeric" required /></label>
    {#if error}<p class="border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>{/if}
    <div class="flex flex-wrap gap-2"><button class="min-h-11 rounded-sm border border-accent bg-raised px-4 py-2 text-body-sm font-medium text-accent-text hover:bg-[var(--hiero-accent-bg)] disabled:cursor-not-allowed disabled:opacity-60" disabled={busy}>Save profile</button>{#if provider}<button class="min-h-11 rounded-sm border border-default bg-surface px-4 py-2 text-body-sm text-primary hover:bg-raised disabled:cursor-not-allowed disabled:opacity-60" type="button" onclick={onCheck} disabled={busy}>Check connection</button><button class="min-h-11 rounded-sm border border-default bg-surface px-4 py-2 text-body-sm text-primary hover:bg-raised disabled:cursor-not-allowed disabled:opacity-60" type="button" onclick={onRefreshModels} disabled={busy}>Refresh models</button>{/if}</div>
  </form>
  {#if provider}<section class="mt-6 border-t border-default pt-5"><h3 class="text-h3">Discovered models</h3>{#if models.length}<ul class="mt-3 grid gap-1 text-mono text-secondary">{#each models as model (model)}<li>{model}</li>{/each}</ul>{:else}<p class="mt-3 text-body-sm text-secondary">No cached models. Refresh after testing the connection.</p>{/if}</section><button class="mt-6 min-h-11 rounded-sm border border-danger bg-[var(--hiero-danger-bg)] px-4 py-2 text-body-sm font-medium text-danger hover:bg-danger hover:text-white disabled:cursor-not-allowed disabled:opacity-60" onclick={onDelete} disabled={busy}>Delete provider</button>{/if}
</dialog>
