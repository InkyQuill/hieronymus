<script lang="ts">
  import type { AdminDashboard } from "../lib/types";
  import { formatReadiness, parseSummary } from "../lib/readiness";
  import { workflowGuide } from "../lib/presentation";
  import TechnicalDetails from "./TechnicalDetails.svelte";

  type Props = { dashboard: AdminDashboard; error?: string; onDream: () => void; busy?: boolean };
  let { dashboard, error = "", onDream, busy = false }: Props = $props();
  const labels: Record<string, string> = { crystals: "Long-term memories", short_term_memories: "Recent memories", series: "Books", sessions: "Reading sessions", dream_runs: "Processing runs", audit_events: "Activity events", lessons: "Writing lessons" };
  const label = (key: string) => labels[key] ?? key.replaceAll("_", " ");
  const dreamState = $derived(String(dashboard.dream_status.state ?? "unknown").toUpperCase());
  const dreamingLabel = $derived(dreamState === "WORKING" ? "Processing memories" : dreamState === "DISABLED" ? "Scheduled processing is off" : dreamState === "IDLE" ? "Waiting for the next run" : "Status unavailable");
  const statLinks: Record<string, string> = {
    audit_events: "/admin/memory?view=Audit%20Log",
    crystals: "/admin/memory?view=Crystals",
    dream_runs: "/admin/memory?view=Dream%20Runs",
    lessons: "/admin/memory?view=Lessons",
    series: "/admin/memory",
    sessions: "/admin/memory?view=Short-Term%20Sessions",
    short_term_memories: "/admin/memory?view=Short-Term%20Memory",
  };
  const workflow = [
    ["concepts", "Concepts"],
    ["terminology_candidates", "Terms"],
    ["rule_crystals", "Rules"],
    ["knowledge_crystals", "Knowledge"],
    ["relations", "Relations"],
    ["reinforcement", "Reinforcement"],
    ["coverage_audit", "Coverage"],
  ] as const;
  const currentPhase = $derived(String(dashboard.dream_status.current_phase ?? ""));
  const currentPhaseIndex = $derived(workflow.findIndex(([phase]) => phase === currentPhase));
  const readiness = $derived(formatReadiness(dashboard.readiness));

  function workflowState(index: number): "complete" | "active" | "pending" {
    if (currentPhaseIndex < 0) return "pending";
    if (index < currentPhaseIndex) return "complete";
    if (index === currentPhaseIndex) return "active";
    return "pending";
  }
</script>

<section class="grid gap-8 lg:grid-cols-[minmax(14rem,18rem)_minmax(0,1fr)]" aria-label="Administration overview">
  <div class="self-start lg:sticky lg:top-24">
    <h1 class="text-display">Your writing memory</h1>
    <p class="mt-3 max-w-prose text-body text-secondary">See what your agent has retained and whether memory processing needs attention.</p>
    <div class="mt-6 flex flex-wrap gap-2 border-t border-default pt-4">
      <a class="inline-flex min-h-11 items-center rounded-sm border border-accent bg-raised px-4 py-2 text-body-sm font-medium text-accent-text no-underline" href="/admin/connect">Connect your agent</a>
      <button class="min-h-11 rounded-sm border border-accent bg-raised px-4 py-2 text-body-sm font-medium text-accent-text hover:bg-[var(--hiero-accent-bg)]" disabled={busy || dreamState === "WORKING"} onclick={onDream}>{dreamState === "WORKING" ? "Processing…" : "Process memories now"}</button>
      <a class="inline-flex min-h-11 items-center rounded-sm border border-default bg-surface px-4 py-2 text-body-sm text-primary no-underline hover:bg-raised" href="/admin/memory">Browse memory</a>
      <a class="inline-flex min-h-11 items-center rounded-sm border border-default bg-surface px-4 py-2 text-body-sm text-primary no-underline hover:bg-raised" href="/config">Memory settings</a>
    </div>
  </div>
  <div class="min-w-0">
    <div class="grid border-y border-default sm:grid-cols-2" aria-label="Memory statistics">
      {#each Object.entries(dashboard.stats) as [name, value] (name)}
        <a class="flex items-center justify-between gap-4 border-b border-default px-4 py-3 text-body-sm no-underline hover:bg-raised" href={statLinks[name] ?? "/admin/memory"}>
          <span class="text-secondary">{label(name)}</span>
          <strong class="text-body font-medium tabular-nums text-primary">{value}</strong>
        </a>
      {/each}
    </div>
    <section class="mt-4 rounded-md border border-default bg-surface p-5" aria-label="Dreaming workflow status">
        <div class="flex flex-wrap items-baseline justify-between gap-4"><h3 class="text-h3">Memory processing · Dreaming</h3><span class="text-body-sm text-secondary">{dreamingLabel}</span></div>
        <details open={dreamState === "WORKING"}><summary class="mt-3 min-h-11 cursor-pointer py-3 text-body-sm text-secondary">Processing stages</summary>
        <ol class="mt-2 grid grid-cols-2 gap-2 xl:grid-cols-7">
          {#each workflow as [phase, name], index (phase)}
            <li class="min-w-0 text-caption {workflowState(index) === 'active' ? 'text-accent-text' : 'text-secondary'}" aria-current={workflowState(index) === "active" ? "step" : undefined}>
              <span class="mb-2 block h-1 w-full {workflowState(index) === 'complete' ? 'bg-accent' : workflowState(index) === 'active' ? 'h-1.5 bg-accent' : 'bg-default'}" aria-hidden="true"></span><span>{name}</span>
            </li>
          {/each}
        </ol></details>
        {#if currentPhase}
          <p class="mt-3 text-caption capitalize text-secondary">{workflowGuide[currentPhase]?.label ?? currentPhase.replaceAll("_", " ")} · {Math.round(Number(dashboard.dream_status.progress ?? 0) * 100)}%</p>
        {:else}
          <p class="mt-3 text-caption text-secondary">{dreamState === "DISABLED" ? "Scheduled processing is off. You can run it manually or enable a schedule in Dreaming settings." : dreamState === "IDLE" ? "Dreaming turns recent memories into lasting knowledge. It runs on the configured schedule." : "Processing status is unavailable."}</p>
        {/if}
        <div class="mt-4 flex flex-wrap gap-4 text-body-sm"><a href="/config/dreaming" class="text-accent-text underline">Processing settings</a><a href="/admin/memory?view=Dream%20Runs" class="text-accent-text underline">View processing history</a></div>
    </section>
    <section class="mt-4 rounded-md border border-default bg-surface p-5" aria-label="Local service status">
      <h3 class="mb-4 text-h3">Local service</h3>
      <p class="mb-4 max-w-[70ch] text-body-sm text-secondary">{readiness.level === "Ready" ? "The service reports that it is ready." : readiness.level === "Degraded" ? "Some memory features need attention. Review the reported problems below and check the AI connection or processing history." : readiness.level === "Starting" ? "The service is starting. Check this page again when startup finishes." : "The service has not reported its readiness. Open technical details to inspect the available status."}</p>
      <dl class="flex flex-wrap gap-x-12 gap-y-4">
        <div><dt class="text-caption text-secondary">Readiness</dt><dd class="mt-1 text-body">{readiness.level}</dd></div>
        <div><dt class="text-caption text-secondary">Recent memories</dt><dd class="mt-1 text-body">{dashboard.short_term_status.pending_count == null ? "Pending count unavailable" : `${Number(dashboard.short_term_status.pending_count)} awaiting processing`}</dd></div>
      </dl>
      {#if readiness.reasons.length > 0}
        <ul class="mt-4 space-y-1 text-body-sm text-secondary" aria-label="Readiness reasons">
          {#each readiness.reasons as reason (reason)}<li>{reason}</li>{/each}
        </ul>
      {/if}
      {#if readiness.level === "Degraded"}<a href="/config" class="mt-4 inline-flex min-h-11 items-center text-body-sm text-accent-text underline">Check AI connections</a>{/if}
      <TechnicalDetails data={{ readiness: parseSummary(dashboard.readiness), dreaming: { state: dashboard.dream_status.state, current_phase: dashboard.dream_status.current_phase, progress: dashboard.dream_status.progress }, recent_memory: { pending_count: dashboard.short_term_status.pending_count } }} label="Technical service status" />
      {#if readiness.providers.length > 0}
        <details class="mt-4"><summary class="min-h-11 cursor-pointer py-3 text-body-sm text-secondary">AI provider checks</summary>
        <ul class="mt-4 grid gap-2" aria-label="Provider readiness">
          {#each readiness.providers as provider (`${provider.provider}:${provider.model}`)}
            <li class="rounded-sm border border-default bg-raised px-3 py-2 text-body-sm">
              <span class="font-medium">{provider.provider} / {provider.model}</span>
              <span class="ml-2 text-secondary">{provider.condition}</span>
              {#if provider.reason}<span class="mt-1 block text-caption text-secondary">{provider.reason}</span>{/if}
            </li>
          {/each}
        </ul></details>
      {/if}
    </section>
    {#if error}<p class="mt-4 border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>{/if}
  </div>
</section>
