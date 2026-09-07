<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { correctionOptions, correctionSelection, submitCorrection, type ClaimTarget, type CorrectionOptions, type Selection } from "../lib/authority";
  let { target, onclose }: { target?: ClaimTarget; onclose: () => void } = $props();
  let options = $state<CorrectionOptions>({ series: [], sources: [] });
  let series = $state(0), source = $state(0), rule = $state(0), claim = $state(0);
  let selection = $state<Selection | null>(null);
  let mode = $state<"rendering" | "invalidate" | "qualify">(untrack(() => target ? "invalidate" : "rendering"));
  let value = $state("");
  let busy = $state(false), error = $state(""), notice = $state(""), applied = $state(false);
  let savedRequest: Record<string, unknown> | null = null;
  let sequence = 0;
  const sources = $derived(options.sources.filter(s => s.series_id === series));
  const chosenSource = $derived(sources.find(s => s.id === source));
  const chosenClaim = $derived(selection?.claims.find(c => c.claim_id === claim));
  const ready = $derived(!busy && !applied && selection !== null && (mode === "rendering" ? !!selection.source && !!value.trim() && (!chosenSource?.rules.length || !!selection.rule) : !!chosenClaim && (mode === "invalidate" || !!value.trim())));
  function scope(app: Record<string, unknown> | undefined) {
    return [app?.volume_key ? `Volume ${app.volume_key}` : "", app?.chapter_key ? `Chapter ${app.chapter_key}` : ""].filter(Boolean).join(" · ") || "Selected story scope";
  }
  function edited() { savedRequest = null; error = ""; notice = ""; }
  async function inspect() {
    const current = ++sequence;
    selection = null; claim = 0; savedRequest = null; applied = false; notice = ""; error = "";
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
<section class="m-5 grid gap-4 rounded-md border border-default bg-surface p-5" aria-label="Correct a memory">
  <div class="flex items-center justify-between"><h3 class="text-h3">{target ? "Correct this memory" : "Correct a rendering"}</h3><button class="min-h-11 rounded-sm border border-default bg-surface px-4 py-2 text-primary hover:bg-raised disabled:opacity-50" onclick={onclose} disabled={busy}>Close correction</button></div>
  <p class="text-body-sm text-secondary">Choose the exact source occurrence or claim. Your correction takes effect as soon as it is applied.</p>
  <label>Book<select class="mt-1 block w-full rounded border border-default bg-surface p-2" bind:value={series} disabled={busy || applied} onchange={() => { source = 0; rule = 0; void inspect(); }}><option value={0}>Choose a book</option>{#each options.series as book (book.id)}<option value={book.id}>{book.title}</option>{/each}</select></label>
  {#if target}<label>Correction<select class="mt-1 block w-full rounded border border-default bg-surface p-2" bind:value={mode} disabled={busy || applied} onchange={edited}><option value="invalidate">This claim is incorrect</option><option value="qualify">Qualify this claim</option></select></label>{/if}
  {#if mode === "rendering"}
    <label>Source occurrence<select class="mt-1 block w-full rounded border border-default bg-surface p-2" bind:value={source} disabled={busy || applied} onchange={() => { rule = 0; void inspect(); }}><option value={0}>Choose an occurrence</option>{#each sources as item (item.id)}<option value={item.id}>{item.context ?? item.selected_text} · {item.chapter ?? "unspecified chapter"} · {item.source_identity.split("/").pop()}</option>{/each}</select></label>
    {#if chosenSource?.rules.length}<label>{applied ? "Previous rendering (frozen selection)" : "Current rendering to replace"}<select class="mt-1 block w-full rounded border border-default bg-surface p-2" bind:value={rule} disabled={busy || applied} onchange={() => void inspect()}><option value={0}>Choose the current rendering</option>{#each chosenSource.rules as current (current.id)}<option value={current.id}>{current.canonical}</option>{/each}</select></label>{/if}
    {#if selection?.source}<p class="text-body-sm">Selected source: <strong>{selection.source.selected_text}</strong> · {scope(selection.source.binding.applicability)}</p>{/if}
  {:else if selection}
    <fieldset class="grid gap-2"><legend class="mb-2 font-medium">Choose the claim to correct</legend>{#each selection.claims as item (item.claim_id)}<label class="flex gap-2"><input type="radio" name="correction-claim" value={item.claim_id} bind:group={claim} disabled={busy || applied} onchange={edited}/><span>{item.text}<small class="block text-secondary">{scope(item.applicability)}</small></span></label>{/each}</fieldset>
    {#if selection.claims.length === 0}<p>No individually bound claims are available for this record.</p>{/if}
  {/if}
  {#if mode !== "invalidate"}<label>{mode === "rendering" ? "Correct rendering" : "Qualification"}<textarea class="mt-1 block w-full rounded border border-default bg-surface p-2" bind:value disabled={busy || applied} oninput={edited}></textarea></label>{/if}
  {#if !busy && options.series.length === 0}<p>No bound book context is available for this record.</p>{/if}
  {#if error}<p role="alert" class="text-danger">{error}</p><button class="min-h-11 rounded-sm border border-default bg-surface px-4 py-2 text-primary hover:bg-raised disabled:opacity-50" onclick={() => void (options.series.length ? inspect() : initialize())} disabled={busy}>Refresh selection</button>{/if}
  {#if notice}<p role="status" class="text-body-sm">{notice}</p>{/if}
  <button class="min-h-11 rounded-sm border border-accent bg-accent px-4 py-2 font-medium text-primary disabled:opacity-50" disabled={!ready} onclick={apply}>{busy ? "Working…" : "Apply correction"}</button>
</section>
