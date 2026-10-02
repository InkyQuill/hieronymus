<script lang="ts">
  let { current, saved, busy = false }: { current: unknown; saved: unknown; busy?: boolean } = $props();
  const changed = $derived(JSON.stringify(current) !== JSON.stringify(saved));
</script>

<p role="status" class="mt-3 text-body-sm text-secondary">{busy ? "Saving settings…" : changed ? "Unsaved changes — use Save to apply them." : "Matches saved settings."}</p>
