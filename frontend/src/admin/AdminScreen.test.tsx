import React from "react";
import { describe, expect, it } from "vitest";
import { render } from "ink-testing-library";
import { AdminScreen } from "./AdminScreen.js";

function bootstrap() {
  return {
    views: [
      "Concepts",
      "Renderings",
      "Crystals",
      "Lessons",
      "Short-Term Sessions",
      "Dream Runs",
      "Proposals",
      "Audit Log",
    ],
    default_view: "Crystals",
    stats: {
      series: 1,
      crystals: 1,
      lessons: 0,
      short_term_memories: 0,
      sessions: 0,
      dream_runs: 0,
      pending_proposals: 0,
      audit_events: 0,
    },
    service: { running: false },
    snapshot: {
      view: "Crystals",
      rows: [
        {
          id: 1,
          kind: "concept",
          label: "Guild Ledger",
          status: "active",
          scope: "only-sense-online",
          language_pair: "ja -> ru",
          quality_label: "",
          tags: [],
        },
      ],
      selected: {
        id: 1,
        kind: "concept",
        label: "Guild Ledger",
        status: "active",
        scope: "only-sense-online",
        language_pair: "ja -> ru",
        quality_label: "",
        tags: [],
      },
      detail: {
        title: "Guild Ledger",
        subtitle: "concept",
        body: "Guild ledger detail marker.",
        fields: [],
      },
      filters: [],
    },
  };
}

describe("AdminScreen", () => {
  it("renders views, stats, table row, and detail", () => {
    const app = render(
      <AdminScreen initial={bootstrap()} client={undefined} />,
    );

    expect(app.lastFrame()).toContain("Crystals");
    expect(app.lastFrame()).toContain("series 1");
    expect(app.lastFrame()).toContain("Guild Ledger");
    expect(app.lastFrame()).toContain("Guild ledger detail marker.");
  });

  it("shows crystal commands for crystal view", () => {
    const app = render(
      <AdminScreen initial={bootstrap()} client={undefined} showCommands />,
    );

    expect(app.lastFrame()).toContain("reinforce");
    expect(app.lastFrame()).toContain("delete");
    expect(app.lastFrame()).not.toContain("approve");
  });
});
