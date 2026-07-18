<script lang="ts">
  import type { AdminDashboard } from "../lib/types";

  type Props = { dashboard: AdminDashboard; error?: string; onDream: () => void };
  let { dashboard, error = "", onDream }: Props = $props();
  const label = (key: string) => key.replaceAll("_", " ");
  const statLinks: Record<string, string> = {
    audit_events: "/admin/memory?view=Audit%20Log",
    crystals: "/admin/memory?view=Crystals",
    dream_runs: "/admin/memory?view=Dream%20Runs",
    lessons: "/admin/memory?view=Lessons",
    pending_proposals: "/admin/memory?view=Proposals",
    series: "/admin/memory?view=Concepts",
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

  function workflowState(index: number): "complete" | "active" | "pending" {
    if (currentPhaseIndex < 0) return "pending";
    if (index < currentPhaseIndex) return "complete";
    if (index === currentPhaseIndex) return "active";
    return "pending";
  }
</script>

<section class="grid gap-8 lg:grid-cols-[minmax(14rem,18rem)_minmax(0,1fr)]" aria-label="Administration overview">
  <div class="self-start lg:sticky lg:top-24">
    <p class="mb-4 inline-block rounded-full border border-accent bg-[var(--hiero-accent-bg)] px-2.5 py-0.5 text-eyebrow uppercase tracking-[0.12em] text-accent-text">{dashboard.header.version}</p>
    <h2 class="text-display">{dashboard.header.product}</h2>
    <p class="mt-3 max-w-prose text-body text-secondary">{dashboard.header.tagline}</p>
    <div class="mt-6 flex flex-wrap gap-2 border-t border-default pt-4">
      <button class="min-h-11 rounded-sm border border-accent bg-raised px-4 py-2 text-body-sm font-medium text-accent-text hover:bg-[var(--hiero-accent-bg)]" onclick={onDream}>Run Dreaming now</button>
      <a class="inline-flex min-h-11 items-center rounded-sm border border-default bg-surface px-4 py-2 text-body-sm text-primary no-underline hover:bg-raised" href="/admin/memory">Open memory views</a>
      <a class="inline-flex min-h-11 items-center rounded-sm border border-default bg-surface px-4 py-2 text-body-sm text-primary no-underline hover:bg-raised" href="/config">Open configuration</a>
    </div>
  </div>
  <div class="min-w-0">
    <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4" aria-label="Memory statistics">
      {#each Object.entries(dashboard.stats) as [name, value] (name)}
        <a class="rounded-md border border-default bg-surface p-4 no-underline transition hover:-translate-y-px hover:border-accent" href={statLinks[name] ?? "/admin/memory"}>
          <strong class="block text-mono text-2xl text-accent-text">{value}</strong>
          <span class="mt-1 block text-eyebrow uppercase tracking-[0.08em] text-secondary">{label(name)}</span>
        </a>
      {/each}
    </div>
    <section class="mt-4 rounded-md border border-default bg-surface p-5" aria-label="Dreaming workflow status">
        <div class="flex items-baseline justify-between gap-4"><h3 class="text-h3">Dreaming workflow</h3><span class="text-caption text-secondary">{String(dashboard.dream_status.state ?? "unknown")}</span></div>
        <ol class="mt-5 grid grid-cols-2 gap-2 sm:grid-cols-7">
          {#each workflow as [phase, name], index (phase)}
            <li class="min-w-0 text-caption {workflowState(index) === 'complete' ? 'text-secondary' : workflowState(index) === 'active' ? 'text-accent-text' : 'text-tertiary'}" aria-current={workflowState(index) === "active" ? "step" : undefined}>
              <span class="mb-2 block h-1 w-full {workflowState(index) === 'complete' ? 'bg-accent' : workflowState(index) === 'active' ? 'h-1.5 bg-accent' : 'bg-default'}" aria-hidden="true"></span><span>{name}</span>
            </li>
          {/each}
        </ol>
        {#if currentPhase}
          <p class="mt-3 text-caption capitalize text-secondary">{currentPhase.replaceAll("_", " ")} · {Math.round(Number(dashboard.dream_status.progress ?? 0) * 100)}%</p>
        {:else}
          <p class="mt-3 text-caption text-secondary">Ready for the next run.</p>
        {/if}
    </section>
    <section class="mt-4 rounded-md border border-default bg-surface p-5"><h3 class="mb-4 text-h3">Local service</h3><dl class="flex flex-wrap gap-x-12 gap-y-4"><div><dt class="text-caption text-secondary">Dreaming</dt><dd class="mt-1 text-body">{String(dashboard.dream_status.state ?? "unknown")}</dd></div>{#if dashboard.dream_status.current_phase}<div><dt class="text-caption text-secondary">Phase</dt><dd class="mt-1 text-body">{String(dashboard.dream_status.current_phase)} · {Math.round(Number(dashboard.dream_status.progress ?? 0) * 100)}%</dd></div>{/if}<div><dt class="text-caption text-secondary">Short-term memory</dt><dd class="mt-1 text-body">{String(dashboard.short_term_status.state ?? "unknown")}</dd></div></dl></section>
    {#if error}<p class="mt-4 border-l-2 border-danger bg-[var(--hiero-danger-bg)] px-4 py-3 text-body-sm text-danger">{error}</p>{/if}
  </div>
</section>
