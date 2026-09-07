<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { loadAdminSnapshot, runAdminAction } from "../lib/api";
  import type {
    AdminActionBody,
    AdminActionResult,
    AdminCommand,
    AdminDashboard,
    AdminRow,
    AdminSnapshot,
  } from "../lib/types";
  import ActionDialog from "./ActionDialog.svelte";

  type Notice = { message: string; tone: "success" | "error" };
  type Props = { dashboard: AdminDashboard; onNotice: (notice: Notice) => void };

  let { dashboard, onNotice }: Props = $props();

  // The per-view action buttons and their selection requirement come from the
  // daemon's command catalog (`dashboard.command_options` = `ADMIN_COMMANDS`),
  // resolved to the 13 canonical action ids. No hand-maintained frontend map.
  const commands = $derived<AdminCommand[]>(dashboard.command_options ?? []);

  // Destructive actions: the daemon requires `confirmed=true`; the dialog
  // binds it to an explicit checkbox.
  const DESTRUCTIVE = new Set([
    "delete_selected",
    "merge_selected",
    "split_crystal",
  ]);
  // Actions that gather real content/reason before posting.
  const NEEDS_DIALOG = new Set([
    "add_memory",
    "edit_memory",
    "merge_selected",
    "split_crystal",
    "approve_proposal",
    "reject_proposal",
    "delete_selected",
  ]);

  const requestedView =
    new URLSearchParams(window.location.search).get("view") ?? "";
  const defaultView = $derived(
    dashboard.views.includes(requestedView)
      ? requestedView
      : dashboard.views.includes("Crystals")
        ? "Crystals"
        : (dashboard.views[0] ?? ""),
  );
  let selectedView = $state("");
  let selectedIds = $state<Array<string | number>>([]);
  let snapshot = $state.raw<AdminSnapshot["snapshot"] | null>(null);
  let loading = $state(false);
  let error = $state("");
  let runningAction = $state<string | null>(null);
  let dialogCommand = $state<AdminCommand | null>(null);
  let dialogError = $state("");
  let inspection = $state.raw<AdminActionResult | null>(null);

  function commandsFor(view: string): AdminCommand[] {
    return commands.filter((command) => command.views.includes(view));
  }

  function actionable(command: AdminCommand, row: AdminRow | null): boolean {
    if (runningAction !== null) return false;
    return command.requires_selection ? row !== null : true;
  }

  function applySnapshot(next: AdminSnapshot["snapshot"]) {
    snapshot = next;
    selectedIds = selectedIds.filter((id) => next.rows.some((row) => row.id === id));
  }

  // Every load() call takes the next sequence number; a response whose number
  // is no longer current is dropped so a slow earlier fetch can never
  // overwrite newer state.
  let loadSequence = 0;

  async function load(view: string, selectedId?: string | number) {
    const sequence = ++loadSequence;
    if (selectedView !== view) selectedIds = [];
    selectedView = view;
    loading = true;
    error = "";
    inspection = null;
    try {
      const next = (await loadAdminSnapshot(view, selectedId)).snapshot;
      if (sequence !== loadSequence) return;
      applySnapshot(next);
    } catch (reason) {
      if (sequence !== loadSequence) return;
      error = reason instanceof Error ? reason.message : String(reason);
    } finally {
      if (sequence === loadSequence) loading = false;
    }
  }

  // Admin events are hints, not state: several may land while one refresh
  // fetch is still in flight. Coalesce them into exactly one pending
  // follow-up load (the App-level refreshSection already collapses the
  // dashboard fetch the same way) so an event storm never fans out into
  // overlapping snapshot requests.
  let refreshInFlight: Promise<void> | null = null;
  let refreshQueued = false;

  function refreshCurrentView() {
    const view = untrack(() => selectedView);
    if (!view) return;
    if (refreshInFlight) {
      refreshQueued = true;
      return;
    }
    refreshInFlight = (async () => {
      do {
        refreshQueued = false;
        // Re-read the selected row each pass so a selection change during the
        // in-flight fetch is honored by the coalesced follow-up.
        await load(view, untrack(() => snapshot?.selected?.id));
      } while (refreshQueued);
    })().finally(() => {
      refreshInFlight = null;
    });
  }

  // Re-load the current view when the parent hands down a fresh admin
  // dashboard object (an admin event, a completed action). The selected row is
  // preserved by its stable `id` so a background refresh never loses the
  // reader's place. The first effect run only records the initial prop.
  let dashboardSeen = false;
  $effect(() => {
    void dashboard;
    if (!dashboardSeen) {
      dashboardSeen = true;
      return;
    }
    refreshCurrentView();
  });

  async function post(command: AdminCommand, body: AdminActionBody) {
    runningAction = command.id;
    dialogError = "";
    error = "";
    try {
      const result = await runAdminAction(command.id, body);
      // The action result is authoritative: invalidate any in-flight load so
      // it cannot overwrite this snapshot.
      loadSequence += 1;
      loading = false;
      selectedIds = [];
      applySnapshot(result.snapshot);
      dialogCommand = null;
      if (result.provenance || result.reasons || result.review || result.run) {
        inspection = result;
      } else {
        inspection = null;
      }
      onNotice({ message: result.result.message, tone: "success" });
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason);
      if (dialogCommand) dialogError = message;
      else error = message;
      onNotice({ message, tone: "error" });
    } finally {
      runningAction = null;
    }
  }

  function toggleSelection(row: AdminRow, checked: boolean) {
    selectedIds = checked ? [...selectedIds, row.id] : selectedIds.filter((id) => id !== row.id);
    if (checked && !snapshot?.selected) void load(selectedView, row.id);
  }

  function start(command: AdminCommand) {
    const row = snapshot?.selected ?? null;
    if (command.requires_selection && !row) return;
    if (NEEDS_DIALOG.has(command.id)) {
      dialogError = "";
      dialogCommand = command;
      return;
    }
    const body: AdminActionBody = { view: selectedView };
    if (row) body.id = row.id;
    if (command.id === "run_manual_dreaming") body.all = true;
    if (command.id === "review_dream_output" && row) body.run_id = row.id;
    void post(command, body);
  }

  onMount(() => {
    if (defaultView) void load(defaultView);
  });
</script>

<section
  class="grid gap-8 lg:grid-cols-[minmax(14rem,18rem)_minmax(0,1fr)]"
  aria-label="Memory views"
>
  <div class="self-start lg:sticky lg:top-24">
    <p
      class="mb-4 inline-block rounded-full border border-accent bg-[var(--hiero-accent-bg)] px-2.5 py-0.5 text-eyebrow uppercase tracking-[0.12em] text-accent-text"
    >
      Memory administration
    </p>
    <h2 class="text-display">Memory views</h2>
    <p class="mt-3 max-w-prose text-body text-secondary">
      Find a record, read its context, then curate only the memory that needs
      attention.
    </p>
    <div class="mt-6 border-t border-default pt-4 text-caption text-secondary">
      {snapshot?.rows.length ?? 0} records
    </div>
  </div>
  <div class="min-w-0">
    <nav
      class="mb-6 flex flex-wrap gap-1 border-b border-default pb-3"
      aria-label="Memory view selector"
    >
      {#each dashboard.views as view (view)}
        <button
          class="min-h-11 border-b-2 px-3 py-2 text-body-sm {selectedView === view
            ? 'border-accent text-accent-text'
            : 'border-transparent text-secondary hover:bg-raised hover:text-primary'}"
          onclick={() => void load(view)}>{view}</button
        >
      {/each}
    </nav>
    {#if error}<p class="mb-5 border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>{/if}
    {#if loading}
      <p class="text-body text-secondary">Loading {selectedView}…</p>
    {:else if snapshot}
      <div class="grid gap-6 lg:grid-cols-[minmax(0,1.45fr)_minmax(20rem,.8fr)]">
        <div
          class="overflow-x-auto rounded-md border border-default"
          aria-label={`${selectedView} records`}
        >
          {#if snapshot.rows.length}
            <table class="data-table min-w-[42rem] text-left"
              ><thead class="bg-surface"
                ><tr
                  ><th class="border-b border-default px-4 py-3 text-eyebrow">Select</th><th
                    class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary"
                    >Record</th
                  ><th
                    class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary"
                    >Kind</th
                  ><th
                    class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary"
                    >Status</th
                  ><th
                    class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary"
                    >Scope</th
                  ></tr
                ></thead
              ><tbody>
                {#each snapshot.rows as row (row.id)}
                  <tr
                    class="cursor-pointer border-b border-default last:border-b-0 hover:[&>td]:bg-raised {snapshot.selected
                      ?.id === row.id
                      ? '[&>td]:bg-raised [&>td:first-child]:border-l-2 [&>td:first-child]:border-l-accent'
                      : ''}"
                    ><td class="px-4 py-3">
                      <input type="checkbox" aria-label={`Select ${row.label}`}
                        checked={selectedIds.includes(row.id)}
                        onchange={(event) => toggleSelection(row, event.currentTarget.checked)} />
                    </td><td class="px-4 py-3 text-body">
                      <button class="w-full text-left" onclick={() => void load(selectedView, row.id)}
                      ><strong class="block font-medium">{row.label}</strong
                      ><small class="mt-1 block text-caption text-secondary"
                        >{row.language_pair}</small
                      ></button></td
                    ><td class="px-4 py-3 text-body-sm text-secondary"
                      >{row.kind}</td
                    ><td class="px-4 py-3 text-body-sm text-secondary"
                      >{row.status}</td
                    ><td class="px-4 py-3 text-body-sm text-secondary"
                      >{row.scope}</td
                    ></tr
                  >
                {/each}
              </tbody></table
            >
          {:else}
            <table class="data-table"
              ><tbody
                ><tr
                  ><td class="px-4 py-12 text-center text-body text-secondary"
                    >No {selectedView.toLowerCase()} yet. {snapshot.detail
                      .subtitle}</td
                  ></tr
                ></tbody
              ></table
            >
          {/if}
        </div>
        <aside
          class="flex min-h-[22.5rem] flex-col overflow-hidden rounded-md border border-default bg-surface"
          aria-label="Selected memory record"
        >
          {#if snapshot.selected}
            <div class="p-5">
              <p class="text-eyebrow uppercase tracking-[0.08em] text-accent-text">
                {snapshot.selected.kind} · {snapshot.selected.status}
              </p>
              <h3 class="mt-1 text-h3">{snapshot.detail.title}</h3>
              <p class="mt-1 text-body-sm text-secondary">
                {snapshot.detail.subtitle}
              </p>
            </div>
            {#if snapshot.detail.body}<pre
                class="mx-5 min-h-30 overflow-auto border-l-[3px] border-accent bg-raised p-4 font-serif text-[15px] leading-relaxed whitespace-pre-wrap"
                >{snapshot.detail.body}</pre
              >{/if}
            {#if snapshot.detail.fields.length}<dl class="grid gap-2 p-5">
                {#each snapshot.detail.fields as [name, value] (name)}<div
                    class="border-t border-default pt-2"
                  >
                    <dt class="text-caption text-secondary">{name}</dt>
                    <dd class="mt-1 break-words text-mono">{value}</dd>
                  </div>{/each}
              </dl>{/if}
            {#if commandsFor(selectedView).length}<div
                class="mt-auto border-t border-default p-5"
              >
                <h4 class="mb-2 text-body-sm font-medium">Actions</h4>
                <div class="flex flex-wrap gap-2">
                  {#each commandsFor(selectedView) as command (command.id)}<button
                      class="min-h-11 rounded-sm border px-4 py-2 text-body-sm {DESTRUCTIVE.has(
                        command.id,
                      )
                        ? 'border-danger bg-raised text-danger hover:bg-[var(--hiero-danger-bg)]'
                        : 'border-default bg-surface text-primary hover:bg-raised'} disabled:cursor-not-allowed disabled:opacity-50"
                      title={command.hint}
                      disabled={!actionable(command, snapshot.selected)}
                      onclick={() => start(command)}
                      >{runningAction === command.id
                        ? "Working…"
                        : command.label}</button
                    >{/each}
                </div>
              </div>{/if}
            {#if inspection}<div
                class="m-5 rounded-sm border border-default bg-raised p-4 text-body-sm"
                aria-live="polite"
              >
                <h4 class="mb-2 font-medium">{inspection.result.message}</h4>
                <pre class="overflow-auto text-mono text-caption whitespace-pre-wrap">{JSON.stringify(
                    inspection.provenance ??
                      inspection.reasons ??
                      inspection.review ??
                      inspection.run,
                    null,
                    2,
                  )}</pre>
              </div>{/if}
          {:else}
            <div class="p-5">
              <p class="text-body-sm text-secondary">
                Select a record to view its source, status, and available
                actions.
              </p>
              {#if commandsFor(selectedView).some((command) => !command.requires_selection)}
                <div class="mt-4 flex flex-wrap gap-2">
                  {#each commandsFor(selectedView).filter((command) => !command.requires_selection) as command (command.id)}
                    <button
                      class="min-h-11 rounded-sm border border-default bg-surface px-4 py-2 text-body-sm text-primary hover:bg-raised disabled:opacity-50"
                      title={command.hint}
                      disabled={runningAction !== null}
                      onclick={() => start(command)}
                      >{runningAction === command.id
                        ? "Working…"
                        : command.label}</button
                    >
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
        </aside>
      </div>
    {/if}
  </div>
</section>

{#if dialogCommand}
  <ActionDialog
    command={dialogCommand}
    view={selectedView}
    row={snapshot?.selected ?? null}
    {selectedIds}
    currentText={snapshot?.detail.body ?? ""}
    busy={runningAction !== null}
    error={dialogError}
    onSubmit={(body) => void post(dialogCommand!, body)}
    onClose={() => {
      dialogCommand = null;
      dialogError = "";
    }}
  />
{/if}
