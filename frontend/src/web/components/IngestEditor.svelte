<script lang="ts">
  import { onMount } from "svelte";
  import type { IngestSettings } from "../lib/types";

  type Props = { initial: IngestSettings; busy?: boolean; error?: string; onSave: (settings: IngestSettings) => void };
  let { initial, busy = false, error = "", onSave }: Props = $props();
  let settings = $state<IngestSettings>({
    short_memory: { warning_sentence_count: 6, rejection_sentence_count: 30, warning_symbol_count: 0, rejection_symbol_count: 0 },
    learn: { max_block_chars: 1200 },
  });
  onMount(() => { settings = structuredClone(initial); });
</script>

<section class="mx-auto max-w-5xl" aria-label="Ingest settings">
  <header class="flex flex-wrap items-start justify-between gap-4 border-b border-default pb-6"><div><h2 class="text-display">Ingest</h2><p class="mt-2 max-w-2xl text-body text-secondary">Set thresholds for incoming memory quality: sentence counts, symbol limits, and block sizes.</p></div><button class="min-h-11 rounded-sm border border-accent bg-raised px-4 py-2 text-body-sm font-medium text-accent-text hover:bg-[var(--hiero-accent-bg)] disabled:cursor-not-allowed disabled:opacity-60" disabled={busy} onclick={() => onSave($state.snapshot(settings))}>Save ingest</button></header>
  <div class="mt-6 grid gap-4 sm:grid-cols-2">
    <label class="grid gap-1.5 text-caption text-secondary">Warning sentence count<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" type="number" min="1" bind:value={settings.short_memory.warning_sentence_count} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Reject after sentences<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" type="number" min="1" bind:value={settings.short_memory.rejection_sentence_count} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Warning symbol count<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" type="number" min="0" bind:value={settings.short_memory.warning_symbol_count} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Reject after symbols<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" type="number" min="0" bind:value={settings.short_memory.rejection_symbol_count} /></label>
    <label class="grid gap-1.5 text-caption text-secondary">Maximum learn block characters<input class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40" type="number" min="1" bind:value={settings.learn.max_block_chars} /></label>
  </div>
  {#if error}<p class="mt-4 border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>{/if}
</section>
