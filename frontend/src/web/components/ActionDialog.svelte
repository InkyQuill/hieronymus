<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import type {
    AdminActionBody,
    AdminCommand,
    AdminRow,
    AdminSplitPart,
  } from "../lib/types";

  type Props = {
    command: AdminCommand;
    view: string;
    /// The currently selected row (for `requires_selection` actions).
    row?: AdminRow | null;
    /// Additional selected ids for multi-row actions (merge/delete).
    selectedIds?: Array<string | number>;
    /// The current text of the selected record (Detail body), used to
    /// prefill `edit_memory`.
    currentText?: string;
    busy?: boolean;
    error?: string;
    onSubmit: (body: AdminActionBody) => void;
    onClose: () => void;
  };

  let {
    command,
    view,
    row = null,
    selectedIds = [],
    currentText = "",
    busy = false,
    error = "",
    onSubmit,
    onClose,
  }: Props = $props();

  const DESTRUCTIVE = new Set([
    "delete_selected",
    "merge_selected",
    "split_crystal",
  ]);
  // The dialog is re-created per action (keyed by the parent `{#if}`).
  const actionId = $derived(command.id);
  const isDestructive = $derived(DESTRUCTIVE.has(command.id));
  const titleId = $derived(`action-dialog-${command.id}`);

  // Draft fields — only the ones the current action needs are shown. Seeded
  // from the props in `onMount` (reading props in a `$state` initializer would
  // only capture their first value and warns).
  let series = $state("");
  let text = $state("");
  let title = $state("");
  // `edit_memory` seeds the title field for display, but for an untitled
  // crystal the seed is an excerpt of the text — so `title` is only sent when
  // its current value differs from the seed (the backend keeps the stored
  // title when `title` is absent). A one-way dirty flag would wrongly send
  // the excerpt after a type-then-revert, so this compares against the seed.
  let seededTitle = "";
  let reason = $state("");
  let confirmed = $state(false);
  // Parts carry a stable id so `removePart` (which filters the array) can't
  // rebind a textarea to the wrong part.
  let nextPartId = 2;
  let parts = $state<Array<{ id: number; title: string; text: string }>>([
    { id: 0, title: "", text: "" },
    { id: 1, title: "", text: "" },
  ]);

  let dialog: HTMLDialogElement;
  let previouslyFocused: HTMLElement | null = null;

  onMount(async () => {
    if (actionId === "edit_memory") {
      text = currentText;
      seededTitle = row ? String(row.label ?? "") : "";
      title = seededTitle;
    }
    previouslyFocused =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    dialog.showModal();
    await tick();
    dialog
      .querySelector<HTMLElement>(
        "textarea, input:not([type=checkbox]):not([disabled]), input[type=checkbox]",
      )
      ?.focus();
  });

  onDestroy(() => previouslyFocused?.focus());

  function addPart() {
    parts = [...parts, { id: nextPartId++, title: "", text: "" }];
  }

  function removePart(id: number) {
    if (parts.length > 2) parts = parts.filter((part) => part.id !== id);
  }

  function targetIds(): Array<string | number> {
    const ids = selectedIds.length
      ? selectedIds
      : row
        ? [row.id]
        : [];
    return ids;
  }

  function build(): AdminActionBody {
    const body: AdminActionBody = { view };
    if (row) body.id = row.id;
    switch (command.id) {
      case "add_memory":
        return { view, series: series.trim(), text: text.trim(), title: title.trim() };
      case "edit_memory": {
        const edit: AdminActionBody = { ...body, text: text.trim() };
        if (title.trim() !== seededTitle.trim()) edit.title = title.trim();
        return edit;
      }
      case "merge_selected":
        return {
          view,
          ids: targetIds(),
          text: text.trim(),
          title: title.trim(),
          confirmed: true,
        };
      case "split_crystal":
        return {
          ...body,
          confirmed: true,
          parts: parts
            .map<AdminSplitPart>((part) =>
              part.title.trim()
                ? { title: part.title.trim(), text: part.text.trim() }
                : part.text.trim(),
            )
            .filter((part) =>
              typeof part === "string" ? part.length > 0 : Boolean(part.text),
            ),
        };
      case "delete_selected":
        return { view, ids: targetIds(), confirmed: true };
      case "approve_proposal":
      case "reject_proposal":
        return { ...body, reason: reason.trim() };
      default:
        return isDestructive ? { ...body, confirmed: true } : body;
    }
  }

  function canSubmit(): boolean {
    if (busy) return false;
    if (isDestructive && !confirmed) return false;
    switch (command.id) {
      case "add_memory":
        return series.trim().length > 0 && text.trim().length > 0;
      case "edit_memory":
        return text.trim().length > 0;
      case "merge_selected":
        return text.trim().length > 0 && targetIds().length >= 2;
      case "split_crystal":
        return (
          parts.filter((part) => part.text.trim().length > 0).length >= 2
        );
      case "delete_selected":
        return targetIds().length > 0;
      case "reject_proposal":
        return reason.trim().length > 0;
      default:
        return true;
    }
  }

  function submit(event: Event) {
    event.preventDefault();
    if (!canSubmit()) return;
    onSubmit(build());
  }
</script>

<dialog
  bind:this={dialog}
  class="editor-dialog text-primary backdrop:bg-black/30 dark:backdrop:bg-black/60"
  aria-labelledby={titleId}
  oncancel={(event) => {
    event.preventDefault();
    onClose();
  }}
>
  <header class="flex items-center justify-between gap-4 border-b border-default pb-4">
    <div>
      <h2 id={titleId} class="text-h2">{command.label}</h2>
      <p class="mt-1 text-body-sm text-secondary">{command.hint}</p>
    </div>
    <button
      class="inline-flex size-11 items-center justify-center rounded-sm border border-default bg-surface text-2xl leading-none text-secondary hover:bg-raised hover:text-primary"
      aria-label="Close dialog"
      onclick={onClose}>&times;</button
    >
  </header>

  <form class="mt-5 grid gap-4" onsubmit={submit}>
    {#if command.id === "add_memory"}
      <label class="grid gap-1.5 text-caption text-secondary" for="action-series">Series slug</label>
      <input
        id="action-series"
        class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40"
        bind:value={series}
        required
        placeholder="my-series"
      />
      <label class="grid gap-1.5 text-caption text-secondary" for="action-title">Title (optional)</label>
      <input
        id="action-title"
        class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40"
        bind:value={title}
      />
      <label class="grid gap-1.5 text-caption text-secondary" for="action-text">Memory text</label>
      <textarea
        id="action-text"
        class="min-h-30 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40"
        bind:value={text}
        rows="5"
        required
      ></textarea>
    {:else if command.id === "edit_memory"}
      <label class="grid gap-1.5 text-caption text-secondary" for="action-title">Title</label>
      <input
        id="action-title"
        class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40"
        bind:value={title}
      />
      <label class="grid gap-1.5 text-caption text-secondary" for="action-text">Memory text</label>
      <textarea
        id="action-text"
        class="min-h-30 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40"
        bind:value={text}
        rows="6"
        required
      ></textarea>
    {:else if command.id === "merge_selected"}
      <p class="text-body-sm text-secondary">
        Merging {targetIds().length || "the selected"} records into one new memory.
      </p>
      <label class="grid gap-1.5 text-caption text-secondary" for="action-title">Merged title (optional)</label>
      <input
        id="action-title"
        class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40"
        bind:value={title}
      />
      <label class="grid gap-1.5 text-caption text-secondary" for="action-text">Merged memory text</label>
      <textarea
        id="action-text"
        class="min-h-30 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40"
        bind:value={text}
        rows="5"
        required
      ></textarea>
    {:else if command.id === "split_crystal"}
      <fieldset class="grid gap-4">
        <legend class="text-caption text-secondary">Resulting memories (at least two)</legend>
        {#each parts as part, index (part.id)}
          <div class="grid gap-1.5 rounded-sm border border-default p-3">
            <label class="text-caption text-secondary" for={`part-title-${part.id}`}>Part {index + 1} title (optional)</label>
            <input
              id={`part-title-${part.id}`}
              class="min-h-11 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40"
              bind:value={part.title}
            />
            <label class="text-caption text-secondary" for={`part-text-${part.id}`}>Part {index + 1} text</label>
            <textarea
              id={`part-text-${part.id}`}
              class="min-h-20 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40"
              bind:value={part.text}
              rows="3"
            ></textarea>
            {#if parts.length > 2}
              <button
                type="button"
                class="justify-self-start text-body-sm text-danger hover:underline"
                onclick={() => removePart(part.id)}>Remove part {index + 1}</button
              >
            {/if}
          </div>
        {/each}
        <button
          type="button"
          class="justify-self-start min-h-11 rounded-sm border border-default bg-surface px-4 py-2 text-body-sm text-primary hover:bg-raised"
          onclick={addPart}>Add another part</button
        >
      </fieldset>
    {:else if command.id === "delete_selected"}
      <p class="text-body-sm text-secondary">
        Deleting {targetIds().length} {targetIds().length === 1 ? "record" : "records"}.
      </p>
      <ul aria-label="Records to delete" class="list-disc pl-5 text-body-sm text-secondary">
        {#each targetIds() as id (id)}
          <li>{#if row?.id === id}{row.label} (ID: {id}){:else}Record ID: {id}{/if}</li>
        {/each}
      </ul>
    {:else if command.id === "approve_proposal" || command.id === "reject_proposal"}
      <label class="grid gap-1.5 text-caption text-secondary" for="action-reason">
        Reason {command.id === "reject_proposal" ? "(required)" : "(optional)"}
      </label>
      <textarea
        id="action-reason"
        class="min-h-20 rounded-sm border border-strong bg-raised px-3 py-2 text-body text-primary focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent/40"
        bind:value={reason}
        rows="3"
        required={command.id === "reject_proposal"}
      ></textarea>
    {:else}
      <p class="text-body-sm text-secondary">
        {#if row}“{row.label}” will be affected.{:else}This action runs against the current view.{/if}
      </p>
    {/if}

    {#if isDestructive}
      <label class="flex items-center gap-2.5 text-body-sm text-danger">
        <input type="checkbox" bind:checked={confirmed} />
        Yes, apply this change to the stored memory.
      </label>
    {/if}

    {#if error}
      <p class="border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>
    {/if}

    <div class="flex flex-wrap gap-2">
      <button
        class="min-h-11 rounded-sm border px-4 py-2 text-body-sm font-medium disabled:cursor-not-allowed disabled:opacity-60 {isDestructive
          ? 'border-danger bg-[var(--hiero-danger-bg)] text-danger hover:bg-danger hover:text-white'
          : 'border-accent bg-raised text-accent-text hover:bg-[var(--hiero-accent-bg)]'}"
        disabled={!canSubmit()}
      >
        {busy ? "Working…" : command.label}
      </button>
      <button
        type="button"
        class="min-h-11 rounded-sm border border-default bg-surface px-4 py-2 text-body-sm text-primary hover:bg-raised"
        onclick={onClose}>Cancel</button
      >
    </div>
  </form>
</dialog>
