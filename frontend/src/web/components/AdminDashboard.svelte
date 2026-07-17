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

<section class="editorial-split" aria-label="Administration overview">
  <div class="lead">
    <p class="eyebrow">{dashboard.header.version}</p>
    <h2>{dashboard.header.product}</h2>
    <p>{dashboard.header.tagline}</p>
    <div class="lead-meta"><button class="btn-primary" onclick={onDream}>Run Dreaming now</button> <a class="btn-secondary" href="/admin/memory">Open memory views</a> <a class="btn-secondary" href="/config">Open configuration</a></div>
  </div>
  <div class="stage">
    <div class="stats" aria-label="Memory statistics">
      {#each Object.entries(dashboard.stats) as [name, value] (name)}
        <a class="card stat-card" href={statLinks[name] ?? "/admin/memory"}><div><strong>{value}</strong><span>{label(name)}</span></div></a>
      {/each}
    </div>
    <section class="card workflow-card" aria-label="Dreaming workflow status">
      <div>
        <div class="workflow-heading"><h3>Dreaming workflow</h3><span>{String(dashboard.dream_status.state ?? "unknown")}</span></div>
        <ol class="workflow-line">
          {#each workflow as [phase, name], index (phase)}
            <li class={workflowState(index)} aria-current={workflowState(index) === "active" ? "step" : undefined}>
              <span class="workflow-marker" aria-hidden="true"></span><span>{name}</span>
            </li>
          {/each}
        </ol>
        {#if currentPhase}
          <p class="workflow-detail">{currentPhase.replaceAll("_", " ")} · {Math.round(Number(dashboard.dream_status.progress ?? 0) * 100)}%</p>
        {:else}
          <p class="workflow-detail">Ready for the next run.</p>
        {/if}
      </div>
    </section>
    <section class="card status-card"><div><h3>Local service</h3><dl><div><dt>Dreaming</dt><dd>{String(dashboard.dream_status.state ?? "unknown")}</dd></div>{#if dashboard.dream_status.current_phase}<div><dt>Phase</dt><dd>{String(dashboard.dream_status.current_phase)} · {Math.round(Number(dashboard.dream_status.progress ?? 0) * 100)}%</dd></div>{/if}<div><dt>Short-term memory</dt><dd>{String(dashboard.short_term_status.state ?? "unknown")}</dd></div></dl></div></section>
    {#if error}<p class="error-msg">{error}</p>{/if}
  </div>
</section>
