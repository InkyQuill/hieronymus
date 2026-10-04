/** Human labels stay separate from the daemon's stable view and phase IDs. */
export const memoryGuide: Record<
  string,
  { label: string; description: string; empty: string }
> = {
  Crystals: {
    label: "Long-term memories",
    description:
      "Concise knowledge learned from your work. Hieronymus calls these crystals. Open a memory to read it, check its sources or correct it.",
    empty:
      "No long-term memories here yet. Dreaming creates them from recent memories; you can also add a memory yourself.",
  },
  Lessons: {
    label: "Writing lessons",
    description:
      "Reusable writing guidance backed by source memories. Check that a lesson still fits your project before relying on it.",
    empty:
      "No writing lessons here yet. Lessons appear as Hieronymus processes your work.",
  },
  Concepts: {
    label: "Ideas and subjects",
    description:
      "Characters, terms and recurring ideas that connect your memories. Open a subject to inspect its context and translation choices.",
    empty:
      "No subjects here yet. Your agent and Dreaming identify them while working with your project.",
  },
  Renderings: {
    label: "Translation choices",
    description:
      "Translation choices remembered by your agent, including recent terminology observations, proposed variants and translation rules. Their status shows how they are used. Correct a choice when your agent gets it wrong.",
    empty:
      "No translation choices saved for this selection yet. Your agent records terminology while working, and Dreaming develops those observations into lasting memories.",
  },
  "Short-Term Memory": {
    label: "Recent memories",
    description:
      "Source material retained from your work with the agent, before or after Dreaming processes it. A completed record has been processed; it is not necessarily correct.",
    empty:
      "No recent memories here. Continue working with your connected agent; it can retain useful context from your project.",
  },
  "Short-Term Sessions": {
    label: "Working sessions",
    description:
      "Groups of recent memories from work with your agent. Open a session to inspect its book, chapter and reading context.",
    empty:
      "No working sessions here yet. Sessions appear when your agent records work with Hieronymus.",
  },
  "Dream Runs": {
    label: "Processing runs",
    description:
      "Dreaming history across all books. Open a run to see its outcome, processing details and any reported failure.",
    empty:
      "No processing runs yet. Start Dreaming from Overview or configure a schedule in settings.",
  },
  "Dream Audits": {
    label: "Processing events",
    description:
      "Detailed events from Dreaming across all books. Use these to investigate which task ran and what the model returned.",
    empty:
      "No processing events recorded yet. Events appear when Dreaming runs.",
  },
  "Audit Log": {
    label: "Change history",
    description:
      "Recorded decisions and changes across all books. Open an event to inspect its reason and the affected record.",
    empty:
      "No changes recorded here yet. This history fills as recorded decisions are made.",
  },
};

export const workflowGuide: Record<
  string,
  { label: string; description: string }
> = {
  concepts: {
    label: "Identify ideas and subjects",
    description:
      "Find recurring characters, terms and ideas in recent memories.",
  },
  terminology_candidates: {
    label: "Suggest terminology",
    description:
      "Find potential terms and translation choices. Suggestions do not override approved terminology.",
  },
  rule_crystals: {
    label: "Learn writing guidance",
    description:
      "Extract reusable writing and translation rules from source memories.",
  },
  knowledge_crystals: {
    label: "Build lasting knowledge",
    description: "Turn source memories into concise long-term knowledge.",
  },
  relations: {
    label: "Connect memories",
    description: "Find relationships between subjects and memories.",
  },
  reinforcement: {
    label: "Review supporting evidence",
    description: "Update memory strength using supporting source material.",
  },
  coverage_audit: {
    label: "Check source coverage",
    description:
      "Check whether the processing run accounted for its source memories.",
  },
};

export function memoryLabel(view: string): string {
  return memoryGuide[view]?.label ?? view;
}
