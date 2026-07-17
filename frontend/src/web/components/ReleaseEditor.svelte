<script lang="ts">
  import { onMount } from "svelte";
  import type { ReleaseSettings } from "../lib/types";

  type Props = { initial: ReleaseSettings; busy?: boolean; error?: string; onSave: (settings: ReleaseSettings) => void };
  let { initial, busy = false, error = "", onSave }: Props = $props();
  let settings = $state<ReleaseSettings>({ update_channel: "stable" });
  onMount(() => { settings = { ...initial }; });
</script>

<section class="mx-auto max-w-5xl" aria-label="Release settings">
  <header class="flex flex-wrap items-start justify-between gap-4 border-b border-default pb-6"><div><h2 class="text-display">Release</h2><p class="mt-2 max-w-2xl text-body text-secondary">Choose which Hieronymus updates this local installation receives.</p></div><button class="min-h-11 rounded-sm border border-accent bg-raised px-4 py-2 text-body-sm font-medium text-accent hover:bg-[var(--hiero-accent-bg)] disabled:cursor-not-allowed disabled:opacity-60" disabled={busy} onclick={() => onSave($state.snapshot(settings))}>Save release</button></header>
  <fieldset class="mt-6 grid gap-3"><legend class="mb-3 text-h3">Update channel</legend><label class="flex min-h-11 items-center gap-3 rounded-sm border border-default bg-surface px-3 py-2 text-body text-primary"><input class="size-4 accent-[var(--hiero-accent)]" type="radio" bind:group={settings.update_channel} value="stable" /> Stable — published releases only</label><label class="flex min-h-11 items-center gap-3 rounded-sm border border-default bg-surface px-3 py-2 text-body text-primary"><input class="size-4 accent-[var(--hiero-accent)]" type="radio" bind:group={settings.update_channel} value="dev" /> Development — updates from main</label></fieldset>
  {#if error}<p class="mt-4 border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>{/if}
</section>
