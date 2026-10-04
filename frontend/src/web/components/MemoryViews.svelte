<script lang="ts">
  import { CircleHelp } from "@lucide/svelte";
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
  import TechnicalDetails from "./TechnicalDetails.svelte";
  import { memoryGuide, memoryLabel } from "../lib/presentation";
  import { compactSourceLocations } from "../lib/source-locations";

  function sourceLocations(value: string) {
    const occurrences = new Map<string, number>();
    return compactSourceLocations(value).map(location => {
      const occurrence = (occurrences.get(location) ?? 0) + 1;
      occurrences.set(location, occurrence);
      return { location, key: `${location}:${occurrence}` };
    });
  }

  type Notice = { message: string; tone: "success" | "error" };
  type Props = { dashboard: AdminDashboard; bookHeader?: HTMLDivElement; onNotice: (notice: Notice) => void };

  let { dashboard, onNotice, bookHeader }: Props = $props();

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
    selectedIds = []; snapshot = null; loadedSnapshot = null; correction = null; dialogCommand = null;
    try { localStorage.setItem("hieronymus.memory.series", value); } catch { /* Storage may be disabled. */ }
    const url = new URL(window.location.href);
    if (value) url.searchParams.set("series", value); else url.searchParams.delete("series");
    history.replaceState(null, "", url);
    void load(selectedView);
  }
  let detailPanel = $state<HTMLElement>();
  let selectedView = $state("");
  let selectedIds = $state<Array<string | number>>([]);
  let page = $state(0);
  let loadedPage = 0;
  let loadedSnapshot: AdminSnapshot["snapshot"] | null = null;
  let pageSize = $state(20);
  let loadedPageSize = 20;
  const helpId = $props.id();
  function changePageSize(value: string) {
    pageSize = Number(value); page = 0;
    void load(selectedView);
  }
  const recordCount = $derived(snapshot?.total_count ?? snapshot?.rows.length ?? 0);
  const pageCount = $derived(Math.max(1, Math.ceil(recordCount / pageSize)));
  const visibleRows = $derived(snapshot?.total_count !== undefined ? snapshot.rows : (snapshot?.rows ?? []).slice(Math.min(page, pageCount - 1) * pageSize, (Math.min(page, pageCount - 1) + 1) * pageSize));
  const canCombine = $derived(commandsFor(selectedView).some(command => command.id === "merge_selected"));
  let snapshot = $state.raw<AdminSnapshot["snapshot"] | null>(null);
  let loading = $state(false);
  let error = $state("");
  let runningAction = $state<string | null>(null);
  let dialogCommand = $state<AdminCommand | null>(null);
  let dialogError = $state("");
  let correction = $state<{ target?: ClaimTarget } | null>(null);
  let inspection = $state.raw<AdminActionResult | null>(null);

  function changePage(next: number) {
    page = next;
    if (snapshot?.total_count !== undefined) void load(selectedView);
  }


  async function selectRecord(row: AdminRow) {
    correction = null;
    await load(selectedView, row.id, true);
    if (window.matchMedia?.("(max-width: 1079px)").matches) detailPanel?.scrollIntoView({ block: "start", behavior: "smooth" });
  }

  function commandsFor(view: string): AdminCommand[] {
    const modernRendering = view === "Renderings" && typeof snapshot?.selected?.id === "string";
    return commands.filter((command) => command.views.includes(view) && !(modernRendering && command.requires_selection));
  }

  function correctionTarget(row: AdminRow | null): ClaimTarget | undefined {
    if (!row) return;
    if (["Crystals", "Lessons", "Short-Term Memory"].includes(selectedView)) {
      return { source: selectedView === "Short-Term Memory" ? "short_term" : "crystal", id: Number(row.id) };
    }
    if (selectedView === "Renderings" && typeof row.id === "string") {
      const match = /^(memory|crystal|facet):(\d+)$/.exec(row.id);
      if (match) return { source: match[1] === "memory" ? "short_term" : match[1] === "facet" ? "facet" : "crystal", id: Number(match[2]) };
    }
  }

  function actionable(command: AdminCommand, row: AdminRow | null): boolean {
    if (runningAction !== null) return false;
    return command.requires_selection ? row !== null : true;
  }

  function applySnapshot(next: AdminSnapshot["snapshot"]) {
    // Keep unchanged data and its DOM bindings stable during progress events.
    if (JSON.stringify(snapshot) !== JSON.stringify(next)) snapshot = next;
    loadedSnapshot = next;
    loadedPage = page;
    loadedPageSize = pageSize;
    try { localStorage.setItem("hieronymus.memory.pageSize", String(pageSize)); } catch { /* Optional preference. */ }
    page = Math.min(page, Math.max(0, Math.ceil((next.total_count ?? next.rows.length) / pageSize) - 1));
    selectedIds = selectedIds.filter((id) => next.rows.some((row) => row.id === id));
  }

  // Every load() call takes the next sequence number; a response whose number
  // is no longer current is dropped so a slow earlier fetch can never
  // overwrite newer state.
  let loadSequence = 0;

  async function load(view: string, selectedId?: string | number, background = false) {
    const sequence = ++loadSequence;
    if (selectedView !== view) { selectedIds = []; page = 0; snapshot = null; loadedSnapshot = null; correction = null; inspection = null; }
    selectedView = view;
    loading = !background;
    error = "";
    if (!background) inspection = null;
    try {
      const paging = { limit: pageSize, offset: page * pageSize };
      const next = (await loadAdminSnapshot(view, selectedId, selectedSeries || undefined, paging)).snapshot;
      if (sequence !== loadSequence) return;
      if (paging && next.total_count !== undefined) {
        const lastPage = Math.max(0, Math.ceil(next.total_count / pageSize) - 1);
        if (page > lastPage) {
          page = lastPage;
          await load(view, selectedId, background);
          return;
        }
      }
      applySnapshot(next);
    } catch (reason) {
      if (sequence !== loadSequence) return;
      if (loadedSnapshot?.view === view) { snapshot = loadedSnapshot; page = loadedPage; pageSize = loadedPageSize; }
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
        await load(untrack(() => selectedView), untrack(() => snapshot?.selected?.id), true);
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
    untrack(refreshCurrentView);
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
      if (selectedSeries || result.snapshot.total_count !== undefined || snapshot?.total_count !== undefined) await load(selectedView, result.snapshot.selected?.id);
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
    try { const savedSize = Number(localStorage.getItem("hieronymus.memory.pageSize")); if ([10, 20, 50, 100, 200, 500].includes(savedSize)) pageSize = savedSize; } catch { /* Optional preference. */ }
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
  {#if !globalView}<div {@attach (node) => { const target = bookHeader; if (target) target.appendChild(node); return () => node.remove(); }} class="flex items-center gap-2">
    <label for="memory-series" class="text-caption text-secondary">Book</label>
    <select id="memory-series" class="min-h-11 w-44 max-w-full rounded-sm border border-default bg-surface px-3 text-body text-primary" value={selectedSeries} onchange={(event) => chooseSeries(event.currentTarget.value)} disabled={runningAction !== null}>
      <option value="">All books</option>
      {#each books as book (book.slug)}<option value={book.slug}>{book.title || book.slug}</option>{/each}
    </select>

  </div>
  {/if}
  {#if correction}<div class="col-span-full">{#key correction}<CorrectionForm target={correction.target} onclose={() => correction = null} />{/key}</div>{/if}
  <h1 class="sr-only">{globalView ? "Processing history" : "Your project memory"}</h1>
  <div class="min-w-0">
    <label class="mb-4 grid gap-2 text-body-sm text-secondary sm:hidden">Memory section<select class="min-h-11 max-w-full rounded-sm border border-default bg-surface px-3 text-primary" value={selectedView} disabled={runningAction !== null} onchange={(event) => void load(event.currentTarget.value)}>{#each dashboard.views as view (view)}<option value={view}>{memoryLabel(view)}</option>{/each}</select></label>
    <nav
      class="mb-3 hidden flex-wrap sm:flex gap-1 border-b border-default pb-1"
      aria-label="Memory view selector"
    >
      {#each dashboard.views as view (view)}
        <button
          class="min-h-11 border-b-2 px-3 py-2 text-body-sm {selectedView === view
            ? 'border-accent text-accent-text'
            : 'border-transparent text-secondary hover:bg-raised hover:text-primary'}"
          disabled={runningAction !== null}
          aria-pressed={selectedView === view}
          title={view}
          onclick={() => void load(view)}>{memoryLabel(view)}</button
        >
      {/each}
    </nav>
    <div class="mb-3 flex flex-wrap items-center gap-3">
      <h2 class="text-h3">{memoryLabel(selectedView)}</h2>
      <button class="min-h-11 min-w-11 rounded-sm text-secondary hover:bg-raised" popovertarget={helpId} aria-label={`About ${memoryLabel(selectedView).toLowerCase()}`}><CircleHelp size={18} class="mx-auto" /></button>
      <div id={helpId} popover="auto" class="m-auto max-w-[min(90vw,32rem)] rounded-md border border-default bg-surface p-5 text-body-sm text-primary shadow-lg">
        <h3 class="mb-2 text-h3">{memoryLabel(selectedView)}</h3>
        <p>{memoryGuide[selectedView]?.description ?? "Open a record to inspect its context and source."}</p>
        <p class="mt-3">Your agent remembers automatically. A memory may still be uncertain or outdated. Use “Correct this memory” to correct a statement, or “Correct a rendering” to change an approved translation.</p>
        {#if canCombine}<p class="mt-3">Open a record by its title. Checkboxes select memories to combine.</p>{/if}
      </div>
      {#if !globalView}<button class="ml-auto min-h-11 rounded-sm border border-default px-4 py-2 text-body-sm hover:bg-raised" onclick={() => correction = {}}>Correct a rendering</button>{/if}
    </div>
    {#if error}<p class="mb-5 border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>{/if}
    {#if loading && !snapshot}
      <p class="text-body text-secondary">Loading {memoryLabel(selectedView).toLowerCase()}…</p>
    {:else if snapshot}
      {#if canCombine && selectedIds.length}<p class="mb-3 text-body-sm text-secondary">{selectedIds.length} selected for combining.</p>{/if}
      <nav class="mb-4 flex flex-wrap items-center gap-3" aria-label="Record pages"><span class="text-body-sm text-secondary">Page {Math.min(page + 1, pageCount)} of {pageCount} · {recordCount} records</span><button class="min-h-11 rounded-sm border border-default px-4 py-2 disabled:opacity-50" disabled={page === 0 || loading} onclick={() => changePage(page - 1)}>Previous</button><button class="min-h-11 rounded-sm border border-default px-4 py-2 disabled:opacity-50" disabled={page >= pageCount - 1 || loading} onclick={() => changePage(page + 1)}>Next</button><label class="ml-auto flex items-center gap-2 text-body-sm text-secondary">Rows per page<select class="min-h-11 rounded-sm border border-default bg-surface px-3 text-primary" value={pageSize} disabled={loading} onchange={(event) => changePageSize(event.currentTarget.value)}>{#each [10, 20, 50, 100, 200, 500] as size (size)}<option value={size}>{size}</option>{/each}</select></label></nav>
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
                      <button class="w-full text-left" onclick={() => void selectRecord(row)}
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
                    >{memoryGuide[selectedView]?.empty ?? "No records in this view yet."}{#if selectedSeries && !globalView} Try selecting All books to check other projects.{/if}</td
                  ></tr
                ></tbody
              ></table
            >
          {/if}
        </div>
        <aside
          bind:this={detailPanel}
          class="scroll-mt-8 flex min-w-0 flex-col overflow-hidden rounded-md border border-default bg-surface lg:sticky lg:top-28"
          aria-label="Selected memory record"
          aria-busy={loading}
        >
          {#if snapshot.selected}
            <div class="p-5">
              <h3 class="mt-1 text-h3">{snapshot.detail.title}</h3>
              {#if correctionTarget(snapshot.selected)}<button class="min-h-11 rounded-sm border border-default bg-surface px-4 py-2 text-primary hover:bg-raised disabled:opacity-50 mt-3" onclick={() => correction = { target: correctionTarget(snapshot!.selected) }}>Correct this memory</button>{/if}
              <p class="mt-1 text-body-sm text-secondary">
                {snapshot.detail.subtitle}
              </p>
            </div>
            {#if snapshot.detail.body && selectedView !== "Dream Audits"}<pre
                class="mx-5 min-h-30 max-h-[50vh] overflow-auto border border-default bg-raised p-4 font-serif text-[15px] leading-relaxed whitespace-pre-wrap"
                >{snapshot.detail.body}</pre
              >{/if}
            {#if snapshot.detail.fields.some(([name]) => name === "Source locations")}<section class="mx-5 mb-4" aria-label="Memory sources"><h4 class="text-body-sm font-medium">Sources</h4>{#each snapshot.detail.fields.filter(([name]) => name === "Source locations") as [name, value] (name)}{#each sourceLocations(value) as { location, key } (key)}<p class="mt-1 break-words text-body-sm text-secondary" title={value}>{location.slice(Math.max(location.lastIndexOf("/"), location.lastIndexOf("\\")) + 1)}</p>{/each}{/each}</section>{/if}
            {#if commandsFor(selectedView).length}<div
                class="border-b border-default p-5"
              >
                <h4 class="mb-2 text-body-sm font-medium">Actions</h4>
                <div class="flex flex-wrap gap-2">
                  {#each commandsFor(selectedView).filter(command => ["add_memory", "edit_memory"].includes(command.id)) as command (command.id)}<button
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
                {#if commandsFor(selectedView).some(command => !["add_memory", "edit_memory"].includes(command.id))}<details class="mt-3"><summary class="min-h-11 cursor-pointer py-3 text-body-sm text-secondary">More actions and diagnostics</summary><p class="mb-3 text-body-sm text-secondary">Organize stored records, adjust memory strength or inspect evidence. Use correction above to mark a statement wrong or add context.</p><div class="flex flex-wrap gap-2">                  {#each commandsFor(selectedView).filter(command => !["add_memory", "edit_memory"].includes(command.id)) as command (command.id)}<button
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
</div></details>{/if}
              </div>{/if}
            {#if snapshot.detail.fields.some(([name]) => name !== "Source locations")}<details class="p-5"><summary class="min-h-11 cursor-pointer py-3 text-body-sm text-secondary">Source and record details</summary><dl class="grid gap-2">
                {#each snapshot.detail.fields.filter(([name]) => name !== "Source locations") as [name, value] (name)}<div
                    class="border-t border-default pt-2"
                  >
                    <dt class="text-caption text-secondary">{name}</dt>
                    <dd class="mt-1 break-words text-mono">{value}</dd>
                  </div>{/each}
              </dl></details>{/if}
            <div class="px-5 pb-5"><TechnicalDetails data={{ view: selectedView, record: snapshot.selected, detail: snapshot.detail }} label="Technical record data" /></div>
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
