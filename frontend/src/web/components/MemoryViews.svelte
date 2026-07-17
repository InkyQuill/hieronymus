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

<section class="editorial-split" aria-label="Memory views">
  <div class="lead">
    <p class="eyebrow">Memory administration</p>
    <h2>Memory views</h2>
    <p>Find a record, read its context, then curate only the memory that needs attention.</p>
    <div class="lead-meta">{snapshot?.rows.length ?? 0} records</div>
  </div>
  <div class="stage">
    <nav class="view-tabs" aria-label="Memory view selector">
      {#each dashboard.views as view (view)}
        <button class="tab-btn" class:active={selectedView === view} onclick={() => void load(view)}>{view}</button>
      {/each}
    </nav>
    {#if error}<p class="error-msg">{error}</p>{/if}
    {#if loading}
      <p class="loading">Loading {selectedView}…</p>
    {:else if snapshot}
      <div class="memory-split">
        <div class="table-wrap" aria-label={`${selectedView} records`}>
          {#if snapshot.rows.length}
            <table class="memory-table"><thead><tr><th>Record</th><th>Kind</th><th>Status</th><th>Scope</th></tr></thead><tbody>
              {#each snapshot.rows as row (row.id)}
                <tr class:selected={snapshot.selected?.id === row.id} role="button" tabindex="0" onclick={() => void load(selectedView, row.id)} onkeydown={(event) => { if (event.key === " ") event.preventDefault(); if (event.key === "Enter" || event.key === " ") void load(selectedView, row.id); }}><td><strong>{row.label}</strong><small>{row.language_pair}</small></td><td>{row.kind}</td><td>{row.status}</td><td>{row.scope}</td></tr>
              {/each}
            </tbody></table>
          {:else}
            <table><tbody><tr><td class="empty-cell">No {selectedView.toLowerCase()} yet. {snapshot.detail.subtitle}</td></tr></tbody></table>
          {/if}
        </div>
        <aside class="detail-panel" aria-label="Selected memory record">
          {#if snapshot.selected}
            <div><p class="eyebrow">{snapshot.selected.kind} · {snapshot.selected.status}</p><h3>{snapshot.detail.title}</h3><p class="detail-subtitle">{snapshot.detail.subtitle}</p></div>
            {#if snapshot.detail.body}<pre class="detail-body">{snapshot.detail.body}</pre>{/if}
            {#if snapshot.detail.fields.length}<dl>{#each snapshot.detail.fields as [name, value] (name)}<div><dt>{name}</dt><dd>{value}</dd></div>{/each}</dl>{/if}
            {#if actionsFor(snapshot.selected).length}<div class="detail-actions"><h4>Actions</h4><div>{#each actionsFor(snapshot.selected) as action (action)}<button class={destructiveActions.has(action) ? "btn-danger" : "btn-secondary"} disabled={runningAction !== null} onclick={() => requestAction(action)}>{runningAction === action ? "Working…" : actionLabels[action]}</button>{/each}</div></div>{/if}
            {#if pendingAction}<div class="confirm-action" aria-live="polite"><p><strong>{actionLabels[pendingAction]} “{snapshot.selected.label}”?</strong> This action changes the stored memory.</p><div><button class="btn-danger" disabled={runningAction !== null} onclick={() => void perform(pendingAction, true)}>Confirm {actionLabels[pendingAction]}</button><button class="btn-secondary" disabled={runningAction !== null} onclick={() => { pendingAction = null; }}>Cancel</button></div></div>{/if}
          {:else}
            <div><p class="detail-subtitle">Select a record to view its source, status, and available actions.</p></div>
          {/if}
        </aside>
      </div>
    {/if}
  </div>
</section>
