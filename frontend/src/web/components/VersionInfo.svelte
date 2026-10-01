<script lang="ts">
  import { onMount } from "svelte";
  import { loadVersion } from "../lib/api";
  let version = $state<string | null>(null);
  let loading = $state(true);
  let copied = $state(false);
  let copyError = $state(false);
  onMount(() => {
    void loadVersion().then(value => { version = value.server_version; })
      .catch(() => {}).finally(() => { loading = false; });
  });
  async function copy() {
    if (!version) return;
    try {
      await navigator.clipboard.writeText(`Hieronymus server v${version}`);
      copied = true;
      copyError = false;
    } catch { copyError = true; }
  }
</script>
<div class="flex items-center gap-2 text-body-sm text-secondary">
  <span aria-label="Running server version">Server {version ? `v${version}` : loading ? "checking version…" : "version unavailable"}</span>
  {#if version}
    <button type="button" class="rounded-sm px-2 py-1 hover:bg-raised hover:text-primary" onclick={copy} aria-label="Copy version">Copy</button>
  {/if}
  {#if copied}<span role="status">Copied</span>{/if}
  {#if copyError}<span role="status">Select the version text to copy it.</span>{/if}
</div>
