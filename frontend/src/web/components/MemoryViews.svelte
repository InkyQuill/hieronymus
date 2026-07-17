<script lang="ts">
  import { onMount } from "svelte";
  import { loadAdminSnapshot, runAdminAction } from "../lib/api";
  import type {
    AdminDashboard,
    AdminRow,
    AdminSnapshot,
  } from "../lib/types";

  type Notice = { message: string; tone: "success" | "error" };
  type Props = { dashboard: AdminDashboard; onNotice: (notice: Notice) => void };
  type Action =
    | "reinforce_crystal"
    | "decay_crystal"
    | "deprecate_crystal"
    | "delete_crystal"
    | "approve_proposal"
    | "reject_proposal"
    | "reinforce_concept"
    | "decay_concept"
    | "archive_concept"
    | "remove_short_term_memory"
    | "close_session";

  let { dashboard, onNotice }: Props = $props();

  const actionByView: Partial<Record<string, Action[]>> = {
    Crystals: ["reinforce_crystal", "decay_crystal", "deprecate_crystal", "delete_crystal"],
    Proposals: ["approve_proposal", "reject_proposal"],
    Concepts: ["reinforce_concept", "decay_concept", "archive_concept"],
    "Short-Term Sessions": ["close_session"],
  };
  const destructiveActions = new Set<Action>([
    "deprecate_crystal",
    "delete_crystal",
    "archive_concept",
    "remove_short_term_memory",
    "close_session",
  ]);
  const actionLabels: Record<Action, string> = {
    reinforce_crystal: "Reinforce",
    decay_crystal: "Decay",
    deprecate_crystal: "Deprecate",
    delete_crystal: "Delete",
    approve_proposal: "Approve",
    reject_proposal: "Reject",
    reinforce_concept: "Reinforce",
    decay_concept: "Decay",
    archive_concept: "Archive",
    remove_short_term_memory: "Remove",
    close_session: "Close session",
  };

  const requestedView = new URLSearchParams(window.location.search).get("view") ?? "";
  const defaultView = $derived(
    dashboard.views.includes(requestedView)
      ? requestedView
      : dashboard.views.includes("Crystals") ? "Crystals" : (dashboard.views[0] ?? ""),
  );
  let selectedView = $state("");
  let snapshot = $state.raw<AdminSnapshot["snapshot"] | null>(null);
  let loading = $state(false);
  let error = $state("");
  let runningAction = $state<Action | null>(null);
  let pendingAction = $state<Action | null>(null);

  function actionsFor(row: AdminRow | null): Action[] {
    if (!row) return [];
    return actionByView[selectedView] ?? [];
  }

  function applySnapshot(next: AdminSnapshot["snapshot"]) {
    snapshot = next;
  }

  async function load(view: string, selectedId?: string | number) {
    selectedView = view;
    loading = true;
    error = "";
    pendingAction = null;
    try {
      applySnapshot((await loadAdminSnapshot(view, selectedId)).snapshot);
    } catch (reason) {
      error = reason instanceof Error ? reason.message : String(reason);
    } finally {
      loading = false;
    }
  }

  async function perform(action: Action, confirmed = false) {
    const row = snapshot?.selected;
    if (!row) return;
    runningAction = action;
    error = "";
    try {
      const result = await runAdminAction(action, { id: row.id, confirmed });
      applySnapshot(result.snapshot);
      pendingAction = null;
      onNotice({ message: result.result.message, tone: "success" });
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason);
      error = message;
      onNotice({ message, tone: "error" });
    } finally {
      runningAction = null;
    }
  }

  function requestAction(action: Action) {
    if (destructiveActions.has(action)) {
      pendingAction = action;
      return;
    }
    void perform(action);
  }

  onMount(() => {
    if (defaultView) void load(defaultView);
  });
</script>

<section class="grid gap-8 lg:grid-cols-[minmax(14rem,18rem)_minmax(0,1fr)]" aria-label="Memory views">
  <div class="self-start lg:sticky lg:top-24">
    <p class="mb-4 inline-block rounded-full border border-accent bg-[var(--hiero-accent-bg)] px-2.5 py-0.5 text-eyebrow uppercase tracking-[0.12em] text-accent">Memory administration</p>
    <h2 class="text-display">Memory views</h2>
    <p class="mt-3 max-w-prose text-body text-secondary">Find a record, read its context, then curate only the memory that needs attention.</p>
    <div class="mt-6 border-t border-default pt-4 text-caption text-secondary">{snapshot?.rows.length ?? 0} records</div>
  </div>
  <div class="min-w-0">
    <nav class="mb-6 flex flex-wrap gap-1 border-b border-default pb-3" aria-label="Memory view selector">
      {#each dashboard.views as view (view)}
        <button class="min-h-11 border-b-2 px-3 py-2 text-body-sm {selectedView === view ? 'border-accent text-accent' : 'border-transparent text-secondary hover:bg-raised hover:text-primary'}" onclick={() => void load(view)}>{view}</button>
      {/each}
    </nav>
    {#if error}<p class="mb-5 border-l-2 border-danger bg-raised px-4 py-3 text-body-sm text-danger">{error}</p>{/if}
    {#if loading}
      <p class="text-body text-secondary">Loading {selectedView}…</p>
    {:else if snapshot}
      <div class="grid gap-6 lg:grid-cols-[minmax(0,1.45fr)_minmax(20rem,.8fr)]">
        <div class="overflow-x-auto rounded-md border border-default" aria-label={`${selectedView} records`}>
          {#if snapshot.rows.length}
            <table class="data-table min-w-[42rem] text-left"><thead class="bg-surface"><tr><th class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary">Record</th><th class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary">Kind</th><th class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary">Status</th><th class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary">Scope</th></tr></thead><tbody>
              {#each snapshot.rows as row (row.id)}
                <tr class="cursor-pointer border-b border-default last:border-b-0 hover:[&>td]:bg-raised {snapshot.selected?.id === row.id ? '[&>td]:bg-raised [&>td:first-child]:border-l-2 [&>td:first-child]:border-l-accent' : ''}" role="button" tabindex="0" onclick={() => void load(selectedView, row.id)} onkeydown={(event) => { if (event.key === " ") event.preventDefault(); if (event.key === "Enter" || event.key === " ") void load(selectedView, row.id); }}><td class="px-4 py-3 text-body"><strong class="block font-medium">{row.label}</strong><small class="mt-1 block text-caption text-secondary">{row.language_pair}</small></td><td class="px-4 py-3 text-body-sm text-secondary">{row.kind}</td><td class="px-4 py-3 text-body-sm text-secondary">{row.status}</td><td class="px-4 py-3 text-body-sm text-secondary">{row.scope}</td></tr>
              {/each}
            </tbody></table>
          {:else}
            <table class="data-table"><tbody><tr><td class="px-4 py-12 text-center text-body text-secondary">No {selectedView.toLowerCase()} yet. {snapshot.detail.subtitle}</td></tr></tbody></table>
          {/if}
        </div>
        <aside class="flex min-h-[22.5rem] flex-col overflow-hidden rounded-md border border-default bg-surface" aria-label="Selected memory record">
          {#if snapshot.selected}
            <div class="p-5"><p class="text-eyebrow uppercase tracking-[0.08em] text-accent">{snapshot.selected.kind} · {snapshot.selected.status}</p><h3 class="mt-1 text-h3">{snapshot.detail.title}</h3><p class="mt-1 text-body-sm text-secondary">{snapshot.detail.subtitle}</p></div>
            {#if snapshot.detail.body}<pre class="mx-5 min-h-30 overflow-auto border-l-[3px] border-accent bg-raised p-4 font-serif text-[15px] leading-relaxed whitespace-pre-wrap">{snapshot.detail.body}</pre>{/if}
            {#if snapshot.detail.fields.length}<dl class="grid gap-2 p-5">{#each snapshot.detail.fields as [name, value] (name)}<div class="border-t border-default pt-2"><dt class="text-caption text-secondary">{name}</dt><dd class="mt-1 break-words text-mono">{value}</dd></div>{/each}</dl>{/if}
            {#if actionsFor(snapshot.selected).length}<div class="mt-auto border-t border-default p-5"><h4 class="mb-2 text-body-sm font-medium">Actions</h4><div class="flex flex-wrap gap-2">{#each actionsFor(snapshot.selected) as action (action)}<button class="min-h-11 rounded-sm border px-4 py-2 text-body-sm {destructiveActions.has(action) ? 'border-danger bg-raised text-danger hover:bg-[var(--hiero-danger-bg)]' : 'border-default bg-surface text-primary hover:bg-raised'}" disabled={runningAction !== null} onclick={() => requestAction(action)}>{runningAction === action ? "Working…" : actionLabels[action]}</button>{/each}</div></div>{/if}
            {#if pendingAction}<div class="m-5 border border-danger bg-[var(--hiero-danger-bg)] p-4 text-body-sm" aria-live="polite"><p class="mb-3"><strong>{actionLabels[pendingAction]} “{snapshot.selected.label}”?</strong> This action changes the stored memory.</p><div class="flex flex-wrap gap-2"><button class="min-h-11 rounded-sm border border-danger bg-raised px-4 py-2 text-body-sm text-danger hover:bg-[var(--hiero-danger-bg)]" disabled={runningAction !== null} onclick={() => void perform(pendingAction, true)}>Confirm {actionLabels[pendingAction]}</button><button class="min-h-11 rounded-sm border border-default bg-surface px-4 py-2 text-body-sm text-primary hover:bg-raised" disabled={runningAction !== null} onclick={() => { pendingAction = null; }}>Cancel</button></div></div>{/if}
          {:else}
            <div class="p-5"><p class="text-body-sm text-secondary">Select a record to view its source, status, and available actions.</p></div>
          {/if}
        </aside>
      </div>
    {/if}
  </div>
</section>
