<script lang="ts">
  let { data, label = "Technical details" }: { data: unknown; label?: string } = $props();
  let copied = $state(false);
  let error = $state("");
  const json = $derived(JSON.stringify(data, null, 2) ?? "No data available.");
  async function copy() {
    copied = false;
    error = "";
    try { await navigator.clipboard.writeText(json); copied = true; }
    catch { error = "Copy is unavailable. Select and copy the details below."; }
  }
</script>

<details class="technical-details mt-6 border-t border-default pt-2">
  <summary class="min-h-11 cursor-pointer py-3 text-body-sm text-secondary">{label}</summary>
  <p class="mb-3 max-w-[70ch] text-body-sm text-secondary">Data for this view, with the original field names. Settings include any edits shown above.</p>
  <button type="button" class="mb-3 min-h-11 rounded-sm border border-default px-4 py-2 text-body-sm hover:bg-raised" onclick={copy}>Copy technical details</button>
  {#if copied}<p role="status" class="mb-3 text-body-sm text-secondary">Details copied.</p>{/if}
  {#if error}<p role="alert" class="mb-3 text-body-sm text-danger">{error}</p>{/if}
  <pre class="max-h-[50vh] overflow-auto rounded-sm border border-default bg-raised p-4 text-mono whitespace-pre-wrap break-words">{json}</pre>
</details>
