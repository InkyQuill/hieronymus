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
    | "remove_short_term_memory";

  let { dashboard, onNotice }: Props = $props();

  const actionByView: Partial<Record<string, Action[]>> = {
    Crystals: ["reinforce_crystal", "decay_crystal", "deprecate_crystal", "delete_crystal"],
    Proposals: ["approve_proposal", "reject_proposal"],
    Concepts: ["reinforce_concept", "decay_concept", "archive_concept"],
    "Short-Term Sessions": ["remove_short_term_memory"],
  };
  const destructiveActions = new Set<Action>([
    "deprecate_crystal",
    "delete_crystal",
    "archive_concept",
    "remove_short_term_memory",
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
  };

  const defaultView = $derived(
    dashboard.views.includes("Crystals") ? "Crystals" : (dashboard.views[0] ?? ""),
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

<section class="memory-workspace" aria-label="Memory views">
  <header class="page-header">
    <div>
      <p class="eyebrow">Memory administration</p>
      <h2>Memory views</h2>
      <p>Find a record, read its context, then curate only the memory that needs attention.</p>
    </div>
    <span class="memory-count">{snapshot?.rows.length ?? 0} records</span>
  </header>

  <nav class="view-tabs" aria-label="Memory view selector">
    {#each dashboard.views as view (view)}
      <button class:active={selectedView === view} onclick={() => void load(view)}>{view}</button>
    {/each}
  </nav>

  {#if error}<p class="error">{error}</p>{/if}
  {#if loading}<p>Loading {selectedView}…</p>
  {:else if snapshot}
    <div class="memory-split">
      <section class="memory-table-panel" aria-label={`${selectedView} records`}>
        {#if snapshot.rows.length}
          <table class="memory-table">
            <thead><tr><th>Record</th><th>Kind</th><th>Status</th><th>Scope</th></tr></thead>
            <tbody>
              {#each snapshot.rows as row (row.id)}
                <tr class:selected={snapshot.selected?.id === row.id} onclick={() => void load(selectedView, row.id)}>
                  <td><strong>{row.label}</strong><small>{row.language_pair}</small></td>
                  <td>{row.kind}</td><td>{row.status}</td><td>{row.scope}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <div class="empty"><h3>No {selectedView.toLowerCase()} yet</h3><p>{snapshot.detail.subtitle}</p></div>
        {/if}
      </section>

      <aside class="memory-detail" aria-label="Selected memory record">
        {#if snapshot.selected}
          <p class="eyebrow">{snapshot.selected.kind} · {snapshot.selected.status}</p>
          <h3>{snapshot.detail.title}</h3>
          <p class="memory-subtitle">{snapshot.detail.subtitle}</p>
          {#if snapshot.detail.body}<pre>{snapshot.detail.body}</pre>{/if}
          {#if snapshot.detail.fields.length}
            <dl>{#each snapshot.detail.fields as [name, value] (name)}<div><dt>{name}</dt><dd>{value}</dd></div>{/each}</dl>
          {/if}
          {#if actionsFor(snapshot.selected).length}
            <section class="memory-actions" aria-label="Available actions"><h4>Actions</h4><div>
              {#each actionsFor(snapshot.selected) as action (action)}
                <button class:danger={destructiveActions.has(action)} disabled={runningAction !== null} onclick={() => requestAction(action)}>{runningAction === action ? "Working…" : actionLabels[action]}</button>
              {/each}
            </div></section>
          {/if}
          {#if pendingAction}
            <section class="confirm-action" aria-live="polite"><p><strong>{actionLabels[pendingAction]} “{snapshot.selected.label}”?</strong> This action changes the stored memory.</p><div><button class="danger" disabled={runningAction !== null} onclick={() => void perform(pendingAction, true)}>Confirm {actionLabels[pendingAction]}</button><button disabled={runningAction !== null} onclick={() => { pendingAction = null; }}>Cancel</button></div></section>
          {/if}
        {:else}
          <div class="empty"><h3>Select a record</h3><p>Its source, status, and available actions will appear here.</p></div>
        {/if}
      </aside>
    </div>
  {/if}
</section>
