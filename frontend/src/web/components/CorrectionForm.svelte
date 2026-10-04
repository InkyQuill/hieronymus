<script lang="ts">
  import TechnicalDetails from "./TechnicalDetails.svelte";
  import { onMount, untrack } from "svelte";
  import { correctionOptions, correctionSelection, submitCorrection, type ClaimTarget, type CorrectionOptions, type Selection } from "../lib/authority";
  let { target, onclose }: { target?: ClaimTarget; onclose: () => void } = $props();
  let options = $state<CorrectionOptions>({ series: [], sources: [] });
  let series = $state(0), source = $state(0), rule = $state(0), claim = $state(0);
  let selection = $state<Selection | null>(null);
  let mode = $state<"rendering" | "invalidate" | "qualify">(untrack(() => target ? "invalidate" : "rendering"));
  let value = $state("");
  let claimSearch = $state("");
  let claimPage = $state(0);
  const matchingClaims = $derived((selection?.claims ?? []).filter(item => item.text.toLocaleLowerCase().includes(claimSearch.trim().toLocaleLowerCase())));
  const visibleClaims = $derived(matchingClaims.slice(claimPage * 10, (claimPage + 1) * 10));
  function claimPreview(text: string) {
    const firstSentence = text.split(/(?<=[.!?])\s/)[0];
    return firstSentence.length > 180 ? `${firstSentence.slice(0, 180)}…` : firstSentence;
  }
  let busy = $state(false), error = $state(""), notice = $state(""), applied = $state(false);
  let savedRequest: Record<string, unknown> | null = null;
  let sequence = 0;
  const sources = $derived(options.sources.filter(s => s.series_id === series));
  const chosenSource = $derived(sources.find(s => s.id === source));
  const chosenClaim = $derived(selection?.claims.find(c => c.claim_id === claim));
  const ready = $derived(!busy && !applied && selection !== null && (mode === "rendering" ? !chosenSource?.context_unresolved && !!selection.source && !!value.trim() && (!chosenSource?.rules.length || !!selection.rule) : !!chosenClaim && (mode === "invalidate" || !!value.trim())));
  function scope(app: Record<string, unknown> | undefined) {
    return [app?.volume_key ? `Volume ${app.volume_key}` : "", app?.chapter_key ? `Chapter ${app.chapter_key}` : ""].filter(Boolean).join(" · ") || "Selected story scope";
  }
  function edited() { savedRequest = null; error = ""; notice = ""; }
  async function inspect() {
    const current = ++sequence;
    selection = null; claim = 0; claimPage = 0; claimSearch = ""; savedRequest = null; applied = false; notice = ""; error = "";
    if (!series || (mode === "rendering" && !source)) return;
    busy = true;
    try {
      const next = await correctionSelection({ series_id: series, ...(mode === "rendering" ? { source_evidence_id: source, ...(rule ? { rule_id: rule } : {}) } : { target }) });
      if (current === sequence) selection = next;
    } catch (reason) { if (current === sequence) error = reason instanceof Error ? reason.message : String(reason); }
    finally { if (current === sequence) busy = false; }
  }
  async function initialize() {
    busy = true; error = "";
    try { options = await correctionOptions(target); if (options.series.length === 1) series = options.series[0].id; }
    catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
    if (target && series) await inspect();
  }
  onMount(() => { void initialize(); });
  async function apply() {
    if (!ready || !selection) return;
    const structured = mode === "rendering" ? { kind: mode, canonical: value } : mode === "qualify" ? { kind: mode, qualification: value } : { kind: mode };
    if (!savedRequest) {
      const id = crypto.randomUUID();
      savedRequest = { version: 1, decision_id: id, event_id: id, expected_revision: selection.expected_revision, series_id: selection.series_id, source_language: selection.source_language, target_language: mode === "rendering" ? selection.target_language : null, applicability: mode === "rendering" ? selection.source?.binding.applicability : chosenClaim?.applicability, selected_sources: mode === "rendering" ? [selection.source!.reference] : [], selected_claims: chosenClaim && mode !== "rendering" ? [{ id: chosenClaim.claim_id, revision: chosenClaim.revision }] : [], selected_rule: mode === "rendering" && selection.rule ? { id: selection.rule.id, revision: selection.rule.revision } : null, structured };
    }
    busy = true; error = "";
    try {
      const result = await submitCorrection(savedRequest);
      if (result.Applied || result.Replayed) {
        applied = true;
        notice = mode === "rendering" ? `Correction applied. Current rendering: ${value}` : mode === "invalidate" ? "Correction applied. This claim is now marked incorrect." : `Correction applied. Qualification: ${value}`;
      } else notice = `Not applied: ${(result.Tentative?.reasons ?? result.reasons ?? [result.detail ?? "Selection could not be resolved"]).join(", ")}. Review the selection before trying again.`;
    } catch (reason) { error = reason instanceof Error ? reason.message : String(reason); }
    finally { busy = false; }
  }
</script>
<section class="grid gap-4 bg-surface p-5" aria-label="Correct a memory">
  <div class="flex items-center justify-between"><h3 class="text-h3">{target ? "Correct this memory" : "Correct a rendering"}</h3><button class="min-h-11 rounded-sm border border-default bg-surface px-4 py-2 text-primary hover:bg-raised disabled:opacity-50" onclick={onclose} disabled={busy}>Close correction</button></div>
  {#if !target}<p class="text-body-sm text-secondary">{target ? "Choose the statement you want to correct or add context to." : "Choose the book and source passage, then enter the translation your agent should use."} Your change takes effect as soon as it is applied.</p>{/if}
  {#if !target || options.series.length > 1}<label>Book<select class="mt-1 block w-full rounded border border-default bg-surface p-2" bind:value={series} disabled={busy || applied} onchange={() => { source = 0; rule = 0; void inspect(); }}><option value={0}>Choose a book</option>{#each options.series as book (book.id)}<option value={book.id}>{book.title}</option>{/each}</select></label>{/if}

  {#if mode === "rendering"}
    <label>Source occurrence<select class="mt-1 block w-full rounded border border-default bg-surface p-2" bind:value={source} disabled={busy || applied} onchange={() => { rule = 0; void inspect(); }}><option value={0}>Choose an occurrence</option>{#each sources as item (item.id)}<option value={item.id}>{item.context ?? item.selected_text} · {item.chapter ?? "unspecified chapter"} · {item.source_identity.split("/").pop()}</option>{/each}</select></label>
    {#if chosenSource?.context_unresolved}<p>Current rendering cannot be resolved for this occurrence. Select an occurrence with a resolved story position and viewpoint.</p>{/if}
    {#if chosenSource?.rules.length && !chosenSource.context_unresolved}<label>{applied ? "Previous rendering (frozen selection)" : "Current rendering to replace"}<select class="mt-1 block w-full rounded border border-default bg-surface p-2" bind:value={rule} disabled={busy || applied} onchange={() => void inspect()}><option value={0}>Choose the current rendering</option>{#each chosenSource.rules as current (current.id)}<option value={current.id}>{current.canonical}</option>{/each}</select></label>{/if}
    {#if selection?.source}<p class="text-body-sm">Selected source: <strong>{selection.source.selected_text}</strong> · {scope(selection.source.binding.applicability)}</p>{/if}
  {:else if selection}
    {#if !chosenClaim}
      <h4 class="font-medium">1. Choose a statement</h4>
      <label class="grid gap-1 text-body-sm">Find a statement<input type="search" bind:value={claimSearch} oninput={() => claimPage = 0} class="min-h-11 rounded-sm border border-default bg-surface px-3" placeholder="Search a name, term or phrase" /></label>
      <fieldset class="grid max-h-[38vh] gap-2 overflow-y-auto"><legend class="sr-only">Choose the statement to correct</legend>
        {#each visibleClaims as item (item.claim_id)}<label class="flex cursor-pointer items-start gap-3 border-b border-default py-3 text-body-sm"><input type="radio" name="correction-claim" value={item.claim_id} bind:group={claim} disabled={busy || applied} onchange={edited}/><span>{claimPreview(item.text)}{#if scope(item.applicability) !== "Selected story scope"}<small class="block text-secondary">{scope(item.applicability)}</small>{/if}</span></label>{/each}
      </fieldset>
      {#if !matchingClaims.length && selection.claims.length}<p>No statements match this search.</p>{/if}
      {#if matchingClaims.length > 10}<nav class="flex flex-wrap items-center gap-2" aria-label="Statement pages"><span class="text-body-sm text-secondary">{claimPage + 1} / {Math.ceil(matchingClaims.length / 10)} · {matchingClaims.length} statements</span><button class="min-h-11 border border-default px-3" disabled={claimPage === 0} onclick={() => claimPage -= 1}>Previous statements</button><button class="min-h-11 border border-default px-3" disabled={(claimPage + 1) * 10 >= matchingClaims.length} onclick={() => claimPage += 1}>Next statements</button></nav>{/if}
    {:else}
      <div class="grid gap-2"><h4 class="font-medium">Selected statement</h4><p class="text-body-sm">{claimPreview(chosenClaim.text)}</p><details><summary class="min-h-11 cursor-pointer py-3 text-body-sm text-secondary">Read the complete statement</summary><p class="max-h-64 overflow-auto whitespace-pre-wrap text-body-sm">{chosenClaim.text}</p></details><button class="min-h-11 text-left text-body-sm text-accent-text" disabled={busy || applied} onclick={() => { claim = 0; edited(); }}>Choose another statement</button></div>
    {/if}
    {#if selection.claims.length === 0}<p>This memory has no separately recorded statements to correct.</p>{/if}
  {/if}
  {#if target && chosenClaim}<label>Correction<select class="mt-1 block w-full rounded border border-default bg-surface p-2" bind:value={mode} disabled={busy || applied} onchange={edited}><option value="invalidate">This is wrong or outdated</option><option value="qualify">Add a pointer or context</option></select></label>{/if}
  {#if mode !== "invalidate" && (!target || chosenClaim)}<label>{mode === "rendering" ? "Correct rendering" : "Pointer or context for your agent"}<textarea class="mt-1 block w-full rounded border border-default bg-surface p-2" bind:value disabled={busy || applied} oninput={edited}></textarea></label>{/if}
  {#if !busy && options.series.length === 0}<p>No bound book context is available for this record.</p>{/if}
  {#if error}<p role="alert" class="text-danger">{error}</p><button class="min-h-11 rounded-sm border border-default bg-surface px-4 py-2 text-primary hover:bg-raised disabled:opacity-50" onclick={() => void (options.series.length ? inspect() : initialize())} disabled={busy}>Refresh selection</button>{/if}
  {#if notice}<p role="status" class="text-body-sm">{notice}</p>{/if}
  <TechnicalDetails data={{ selectedClaim: chosenClaim, expectedRevision: selection?.expected_revision, target }} label="Technical correction selection" />
  <button class="min-h-11 rounded-sm border border-accent bg-raised px-4 py-2 font-medium text-accent-text disabled:opacity-50" disabled={!ready} onclick={apply}>{busy ? "Working…" : "Apply correction"}</button>
</section>
