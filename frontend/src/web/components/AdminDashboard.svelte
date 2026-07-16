<script lang="ts">
  import { onMount } from "svelte";
  import { loadAdminSnapshot } from "../lib/api";
  import type { AdminDashboard } from "../lib/types";

  type Props = { dashboard: AdminDashboard; error?: string };
  let { dashboard, error = "" }: Props = $props();

  const label = (key: string) => key.replaceAll("_", " ");
  let selectedView = $state("");
  let rows = $state.raw<Array<Record<string, unknown>>>([]);
  let detail = $state.raw({ title: "", subtitle: "", body: "" });
  let loading = $state(false);
  let snapshotError = $state("");

  async function selectView(view: string) {
    selectedView = view;
    loading = true;
    snapshotError = "";
    try {
      const payload = await loadAdminSnapshot(view);
      rows = payload.snapshot.rows;
      detail = payload.snapshot.detail;
    } catch (reason) {
      snapshotError = reason instanceof Error ? reason.message : String(reason);
    } finally {
      loading = false;
    }
  }

  onMount(() => selectView(dashboard.views.includes("Crystals") ? "Crystals" : dashboard.views[0] ?? ""));
</script>

<section class="settings" aria-label="Administration overview">
  <header class="page-header"><div><p class="eyebrow">{dashboard.header.version}</p><h2>{dashboard.header.product}</h2><p>{dashboard.header.tagline}</p></div><a class="primary" href="/config">Open configuration</a></header>
  <section class="stats" aria-label="Memory statistics">
    {#each Object.entries(dashboard.stats) as [name, value] (name)}<article><strong>{value}</strong><span>{label(name)}</span></article>{/each}
  </section>
  <section class="admin-status"><h3>Local service</h3><dl><div><dt>Dreaming</dt><dd>{String(dashboard.dream_status.state ?? "unknown")}</dd></div><div><dt>Short-term memory</dt><dd>{String(dashboard.short_term_status.state ?? "unknown")}</dd></div></dl></section>
  <section class="admin-status"><h3>Memory views</h3><div class="view-tabs">{#each dashboard.views as view (view)}<button class:active={selectedView === view} onclick={() => selectView(view)}>{view}</button>{/each}</div>{#if loading}<p>Loading {selectedView}…</p>{:else if rows.length}<div class="admin-rows">{#each rows as row, index (String(row.id ?? index))}<article>{#each Object.entries(row).slice(0, 5) as [key, value] (key)}<span><b>{label(key)}:</b> {typeof value === "object" ? JSON.stringify(value) : String(value)}</span>{/each}</article>{/each}</div>{:else}<p>{detail.subtitle || "No rows"}</p>{/if}{#if snapshotError}<p class="error">{snapshotError}</p>{/if}</section>
  {#if error}<p class="error">{error}</p>{/if}
</section>
