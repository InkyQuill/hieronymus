<script lang="ts">
  import { onMount } from "svelte";
  import type { IngestSettings } from "../lib/types";

  type Props = { initial: IngestSettings; busy?: boolean; error?: string; onSave: (settings: IngestSettings) => void };
  let { initial, busy = false, error = "", onSave }: Props = $props();
  let settings = $state<IngestSettings>({
    short_memory: {
      warning_sentence_count: 6,
      rejection_sentence_count: 30,
      warning_symbol_count: 0,
      rejection_symbol_count: 0,
    },
    learn: { max_block_chars: 1200 },
  });
  onMount(() => { settings = structuredClone(initial); });
</script>

<section class="settings" aria-label="Ingest settings">
  <header class="page-header"><div><h2>Ingest</h2><p>Keep incoming memories concise and useful.</p></div><button class="primary" disabled={busy} onclick={() => onSave($state.snapshot(settings))}>Save ingest</button></header>
  <div class="settings-grid">
    <label>Warning sentence count<input type="number" min="1" bind:value={settings.short_memory.warning_sentence_count} /></label>
    <label>Reject after sentences<input type="number" min="1" bind:value={settings.short_memory.rejection_sentence_count} /></label>
    <label>Warning symbol count<input type="number" min="0" bind:value={settings.short_memory.warning_symbol_count} /></label>
    <label>Reject after symbols<input type="number" min="0" bind:value={settings.short_memory.rejection_symbol_count} /></label>
    <label>Maximum learn block characters<input type="number" min="1" bind:value={settings.learn.max_block_chars} /></label>
  </div>
  {#if error}<p class="error">{error}</p>{/if}
</section>
