<script lang="ts">
  import type { AdminDashboard } from "../lib/types";

  type Props = { dashboard: AdminDashboard; error?: string };
  let { dashboard, error = "" }: Props = $props();
  const label = (key: string) => key.replaceAll("_", " ");
</script>

<section class="settings" aria-label="Administration overview">
  <header class="page-header">
    <div><p class="eyebrow">{dashboard.header.version}</p><h2>{dashboard.header.product}</h2><p>{dashboard.header.tagline}</p></div>
    <div class="header-actions"><a class="primary" href="/admin/memory">Open memory views</a><a href="/config">Open configuration</a></div>
  </header>
  <section class="stats" aria-label="Memory statistics">
    {#each Object.entries(dashboard.stats) as [name, value] (name)}<article><strong>{value}</strong><span>{label(name)}</span></article>{/each}
  </section>
  <section class="admin-status"><h3>Local service</h3><dl><div><dt>Dreaming</dt><dd>{String(dashboard.dream_status.state ?? "unknown")}</dd></div><div><dt>Short-term memory</dt><dd>{String(dashboard.short_term_status.state ?? "unknown")}</dd></div></dl></section>
  {#if error}<p class="error">{error}</p>{/if}
</section>
