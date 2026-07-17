<script lang="ts">
  import { onMount } from "svelte";
  import type { ReleaseSettings } from "../lib/types";

  type Props = { initial: ReleaseSettings; busy?: boolean; error?: string; onSave: (settings: ReleaseSettings) => void };
  let { initial, busy = false, error = "", onSave }: Props = $props();
  let settings = $state<ReleaseSettings>({ update_channel: "stable" });
  onMount(() => { settings = { ...initial }; });
</script>

<section class="settings settings-page" aria-label="Release settings">
  <header class="page-header"><div><h2>Release</h2><p>Choose which Hieronymus updates this local installation receives.</p></div><button class="btn-primary" disabled={busy} onclick={() => onSave($state.snapshot(settings))}>Save release</button></header>
  <fieldset><legend>Update channel</legend><label class="radio-label"><input type="radio" bind:group={settings.update_channel} value="stable" /> Stable — published releases only</label><label class="radio-label"><input type="radio" bind:group={settings.update_channel} value="dev" /> Development — updates from main</label></fieldset>
  {#if error}<p class="error-msg">{error}</p>{/if}
</section>
