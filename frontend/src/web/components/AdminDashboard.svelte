<script lang="ts">
  import type { AdminDashboard } from "../lib/types";

  type Props = { dashboard: AdminDashboard; error?: string; onDream: () => void };
  let { dashboard, error = "", onDream }: Props = $props();
  const label = (key: string) => key.replaceAll("_", " ");
</script>

<section class="editorial-split" aria-label="Administration overview">
  <div class="lead">
    <p class="eyebrow">{dashboard.header.version}</p>
    <h2>{dashboard.header.product}</h2>
    <p>{dashboard.header.tagline}</p>
    <div class="lead-meta"><button class="btn-primary" onclick={onDream}>Run Dreaming now</button> <a class="btn-secondary" href="/admin/memory">Open memory views</a> <a class="btn-secondary" href="/config">Open configuration</a></div>
  </div>
  <div class="stage">
    <div class="stats" aria-label="Memory statistics">
      {#each Object.entries(dashboard.stats) as [name, value] (name)}
        <article class="card stat-card"><div><strong>{value}</strong><span>{label(name)}</span></div></article>
      {/each}
    </div>
    <section class="card status-card"><div><h3>Local service</h3><dl><div><dt>Dreaming</dt><dd>{String(dashboard.dream_status.state ?? "unknown")}</dd></div><div><dt>Short-term memory</dt><dd>{String(dashboard.short_term_status.state ?? "unknown")}</dd></div></dl></div></section>
    {#if error}<p class="error-msg">{error}</p>{/if}
  </div>
</section>
