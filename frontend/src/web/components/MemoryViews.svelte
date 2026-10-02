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
  import CorrectionForm from "./CorrectionForm.svelte";
  import type { ClaimTarget } from "../lib/authority";
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
  const books = $derived(dashboard.series_options ?? []);
  let selectedSeries = $state("");
  const globalView = $derived(["Dream Runs", "Dream Audits", "Audit Log"].includes(selectedView));
  function chooseSeries(value: string) {
    selectedSeries = value;
    page = 0;
    selectedIds = []; snapshot = null; correction = null; dialogCommand = null;
    try { localStorage.setItem("hieronymus.memory.series", value); } catch { /* Storage may be disabled. */ }
    const url = new URL(window.location.href);
    if (value) url.searchParams.set("series", value); else url.searchParams.delete("series");
    history.replaceState(null, "", url);
    void load(selectedView);
  }
  let selectedView = $state("");
  let selectedIds = $state<Array<string | number>>([]);
  let page = $state(0);
  const pageSize = 20;
  const pageCount = $derived(Math.max(1, Math.ceil((snapshot?.rows.length ?? 0) / pageSize)));
  const visibleRows = $derived((snapshot?.rows ?? []).slice(Math.min(page, pageCount - 1) * pageSize, (Math.min(page, pageCount - 1) + 1) * pageSize));
  const canCombine = $derived(commandsFor(selectedView).some(command => command.id === "merge_selected"));
  let snapshot = $state.raw<AdminSnapshot["snapshot"] | null>(null);
  let loading = $state(false);
  let error = $state("");
  let runningAction = $state<string | null>(null);
  let dialogCommand = $state<AdminCommand | null>(null);
  let dialogError = $state("");
  let correction = $state<{ target?: ClaimTarget } | null>(null);
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
    page = Math.min(page, Math.max(0, Math.ceil(next.rows.length / pageSize) - 1));
    selectedIds = selectedIds.filter((id) => next.rows.some((row) => row.id === id));
  }

  // Every load() call takes the next sequence number; a response whose number
  // is no longer current is dropped so a slow earlier fetch can never
  // overwrite newer state.
  let loadSequence = 0;

  async function load(view: string, selectedId?: string | number) {
    const sequence = ++loadSequence;
    if (selectedView !== view) { selectedIds = []; page = 0; correction = null; }
    selectedView = view;
    loading = true;
    error = "";
    inspection = null;
    try {
      const next = (await (selectedSeries ? loadAdminSnapshot(view, selectedId, selectedSeries) : loadAdminSnapshot(view, selectedId))).snapshot;
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
        await load(untrack(() => selectedView), untrack(() => snapshot?.selected?.id));
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
      if (selectedSeries) await load(selectedView, result.snapshot.selected?.id);
      else applySnapshot(result.snapshot);
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
    let saved = new URLSearchParams(window.location.search).get("series");
    if (saved === null) { try { saved = localStorage.getItem("hieronymus.memory.series"); } catch { /* Optional preference. */ } }
    if (books.some(book => book.slug === saved)) selectedSeries = saved!;
    if (defaultView) void load(defaultView);
  });
</script>

<section
  class="grid gap-5"
  aria-label="Memory views"
>
  {#if !globalView}<div class="col-span-full">
    <label for="memory-series" class="block text-caption text-secondary">Book</label>
    <select id="memory-series" class="mt-2 min-h-11 w-full rounded-sm border border-default bg-surface px-3 text-body text-primary sm:max-w-sm" value={selectedSeries} onchange={(event) => chooseSeries(event.currentTarget.value)} disabled={runningAction !== null}>
      <option value="">All books</option>
      {#each books as book (book.slug)}<option value={book.slug}>{book.title || book.slug}</option>{/each}
    </select>
    <p class="mt-2 text-caption text-secondary">{globalView ? "This activity log covers all books." : selectedSeries ? "Showing memories for this book. Shared concepts may also appear." : "Choose a book to focus on its memories."}</p>
  </div>
  {/if}
  {#if correction}<div class="col-span-full">{#key correction}<CorrectionForm target={correction.target} onclose={() => correction = null} />{/key}</div>{/if}
  {#if ["Crystals", "Lessons", "Short-Term Memory"].includes(selectedView)}<p class="col-span-full m-5 text-body-sm text-secondary" role="note">A stored memory can still be wrong or outdated. Its status describes storage, not accuracy. Use “Correct this memory” to correct a specific statement.</p>{/if}
  {#if selectedView === "Renderings"}<p class="col-span-full m-5 text-body-sm text-secondary" role="note">These older translation choices are historical records. Use “Correct a rendering” to inspect and change the current approved translation.</p>{/if}
  <header class="flex flex-wrap items-center justify-between gap-3">
    <h2 class="text-h2">{globalView ? "Processing history" : "Your project memory"}</h2>
    {#if !globalView}<button class="min-h-11 rounded-sm border border-default px-4 py-2 text-body-sm hover:bg-raised" onclick={() => correction = {}}>Correct a rendering</button>{/if}
  </header>
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
          disabled={runningAction !== null}
          onclick={() => void load(view)}>{view}</button
        >
      {/each}
    </nav>
    <p class="mb-4 max-w-[70ch] text-body-sm text-secondary">{selectedView === "Crystals" ? "Long-term memories are concise pieces of knowledge your agent has learned. Hieronymus calls these crystals." : selectedView === "Short-Term Memory" ? "Recent memories retain source material before Dreaming processes it into lasting knowledge." : selectedView === "Lessons" ? "Writing lessons capture reusable guidance backed by source memories." : selectedView === "Concepts" ? "Concepts connect recurring ideas, characters and subjects across your memories." : globalView ? "Open a run to review what Hieronymus processed." : "Open a record to inspect its context and source."}</p>
    {#if error}<p class="mb-5 border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>{/if}
    {#if loading}
      <p class="text-body text-secondary">Loading {selectedView}…</p>
    {:else if snapshot}
      {#if canCombine}<p class="mb-3 text-body-sm text-secondary">Open a record by its title. Checkboxes select memories to combine. {selectedIds.length} selected for combining.</p>{/if}
      {#if snapshot.rows.length > pageSize}<nav class="mb-4 flex flex-wrap items-center gap-3" aria-label="Record pages"><span class="text-body-sm text-secondary">Page {Math.min(page + 1, pageCount)} of {pageCount} · {snapshot.rows.length} records</span><button class="min-h-11 rounded-sm border border-default px-4 py-2 disabled:opacity-50" disabled={page === 0 || loading} onclick={() => page -= 1}>Previous</button><button class="min-h-11 rounded-sm border border-default px-4 py-2 disabled:opacity-50" disabled={page >= pageCount - 1 || loading} onclick={() => page += 1}>Next</button></nav>{/if}
      <div class="grid items-start gap-6 lg:grid-cols-[minmax(0,1.45fr)_minmax(20rem,.8fr)]">
        <div
          class="overflow-x-auto rounded-md border border-default"
          aria-label={`${selectedView} records`}
        >
          {#if snapshot.rows.length}
            <table class="data-table min-w-[42rem] text-left"
              ><thead class="bg-surface"
                ><tr
                  >{#if canCombine}<th class="border-b border-default px-4 py-3 text-eyebrow">Combine</th>{/if}<th
                    class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary"
                    >Record</th
                  >{#if selectedView !== "Dream Runs"}<th
                    class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary"
                    >Kind</th>{/if}<th
                    class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary"
                    >Status</th
                  >{#if selectedView !== "Dream Runs"}<th
                    class="border-b border-default px-4 py-3 text-eyebrow uppercase tracking-[0.12em] text-secondary"
                    >Scope</th>{/if}</tr
                ></thead
              ><tbody>
                {#each visibleRows as row (row.id)}
                  <tr
                    class="cursor-pointer border-b border-default last:border-b-0 hover:[&>td]:bg-raised {snapshot.selected
                      ?.id === row.id
                      ? '[&>td]:bg-raised [&>td:first-child]:border-l-2 [&>td:first-child]:border-l-accent'
                      : ''}"
                    >{#if canCombine}<td class="px-4 py-3">
                      <input type="checkbox" aria-label={`Select ${row.label}`}
                        checked={selectedIds.includes(row.id)}
                        onchange={(event) => toggleSelection(row, event.currentTarget.checked)} />
                    </td>{/if}<td class="px-4 py-3 text-body">
                      <button class="w-full text-left" onclick={() => { correction = null; void load(selectedView, row.id); }}
                      ><strong class="block font-medium">{row.label}</strong
                      ><small class="mt-1 block text-caption text-secondary"
                        >{row.language_pair}</small
                      ></button></td
                    >{#if selectedView !== "Dream Runs"}<td class="px-4 py-3 text-body-sm text-secondary"
                      >{row.kind}</td>{/if}<td class="px-4 py-3 text-body-sm text-secondary"
                      >{row.status}</td
                    >{#if selectedView !== "Dream Runs"}<td class="px-4 py-3 text-body-sm text-secondary"
                      >{row.scope}</td>{/if}</tr
                  >
                {/each}
              </tbody></table
            >
          {:else}
            <table class="data-table"
              ><tbody
                ><tr
                  ><td class="px-4 py-12 text-center text-body text-secondary"
                    >No {selectedView.toLowerCase()} match this view. Memories appear here as your agent works with Hieronymus.</td
                  ></tr
                ></tbody
              ></table
            >
          {/if}
        </div>
        <aside
          class="order-first flex flex-col overflow-hidden rounded-md border border-default bg-surface lg:sticky lg:top-4 lg:order-last"
          aria-label="Selected memory record"
        >
          {#if snapshot.selected}
            <div class="p-5">
              <p class="text-eyebrow uppercase tracking-[0.08em] text-accent-text">
                {snapshot.selected.kind} · {snapshot.selected.status}
              </p>
              <h3 class="mt-1 text-h3">{snapshot.detail.title}</h3>
              {#if ["Crystals", "Lessons", "Short-Term Memory"].includes(selectedView)}<button class="min-h-11 rounded-sm border border-default bg-surface px-4 py-2 text-primary hover:bg-raised disabled:opacity-50 mt-3" onclick={() => correction = { target: { source: selectedView === "Short-Term Memory" ? "short_term" : "crystal", id: Number(snapshot!.selected!.id) } }}>Correct this memory</button>{/if}
              <p class="mt-1 text-body-sm text-secondary">
                {snapshot.detail.subtitle}
              </p>
            </div>
            {#if commandsFor(selectedView).length}<div
                class="border-b border-default p-5"
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
            {#if snapshot.detail.body}<pre
                class="mx-5 min-h-30 max-h-[50vh] overflow-auto border border-default bg-raised p-4 font-serif text-[15px] leading-relaxed whitespace-pre-wrap"
                >{snapshot.detail.body}</pre
              >{/if}
            {#if snapshot.detail.fields.length}<details class="p-5"><summary class="min-h-11 cursor-pointer py-3 text-body-sm text-secondary">Source and record details</summary><dl class="grid gap-2">
                {#each snapshot.detail.fields as [name, value] (name)}<div
                    class="border-t border-default pt-2"
                  >
                    <dt class="text-caption text-secondary">{name}</dt>
                    <dd class="mt-1 break-words text-mono">{value}</dd>
                  </div>{/each}
              </dl></details>{/if}
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
    initialSeries={selectedSeries}
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
