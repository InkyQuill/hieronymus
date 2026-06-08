import React from "react";
import { describe, expect, it } from "vitest";
import { render } from "ink-testing-library";
import { App } from "./App.js";
import type { JsonRpcClient } from "../rpc/client.js";

describe("App", () => {
  it("bootstraps and renders the admin screen", async () => {
    const app = render(<App mode="admin" client={fakeClient()} />);

    await waitFor(() => expect(app.lastFrame()).toContain("Hieronymus Admin"));

    expect(app.lastFrame()).toContain("Crystals");
    expect(app.lastFrame()).toContain("series 1");
    expect(app.lastFrame()).toContain("Guild Ledger");
    expect(app.lastFrame()).toContain("Guild ledger detail marker.");
  });
});

function fakeClient(): JsonRpcClient {
  return {
    request: (method: string) => {
      if (method !== "admin.bootstrap") {
        return Promise.reject(new Error(`unexpected request ${method}`));
      }

      return Promise.resolve({
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
      });
    },
  } as unknown as JsonRpcClient;
}

async function waitFor(assertion: () => void) {
  let lastError: unknown;
  for (let attempt = 0; attempt < 20; attempt += 1) {
    try {
      assertion();
      return;
    } catch (error) {
      lastError = error;
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  }
  throw lastError;
}
