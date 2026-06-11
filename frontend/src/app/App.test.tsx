import React from "react";
import { act } from "react";
import { describe, expect, it } from "vitest";
import { testRender } from "@opentui/react/test-utils";
import { App } from "./App.js";
import type { RpcClient } from "../rpc/client.js";

async function setupTest() {
  let node: React.ReactNode = null;
  let setup: Awaited<ReturnType<typeof testRender>> | null = null;
  const ensureSetup = async () => {
    setup ??= await testRender(node, { width: 160, height: 60 });
    return setup;
  };
  const flush = async () => {
    const current = await ensureSetup();
    await act(async () => {
      await current.flush();
    });
  };
  return {
    root: {
      render: (next: React.ReactNode) => {
        node = next;
      },
    },
    mockInput: {},
    flush,
    captureCharFrame: () => setup?.captureCharFrame() ?? "",
    waitFor: async (predicate: () => boolean | Promise<boolean>) => {
      const current = await ensureSetup();
      for (let index = 0; index < 25; index += 1) {
        await act(async () => {
          await Promise.resolve();
          await current.renderOnce();
        });
        if (await predicate()) {
          return;
        }
      }
      throw new Error("Timed out waiting for predicate");
    },
  };
}

describe("App", () => {
  it("bootstraps and renders the admin screen", async () => {
    const { root, flush, captureCharFrame, waitFor } = await setupTest();

    root.render(<App mode="admin" client={fakeClient()} />);
    await flush();

    await waitFor(async () => {
      const frame = captureCharFrame();
      return frame.includes("Hieronymus Admin");
    });

    const output = captureCharFrame();
    expect(output).toContain("Crystals");
    expect(output).toContain("series 1");
    expect(output).toContain("Guild Ledger");
    expect(output).toContain("Guild ledger detail marker.");
  });

  it("renders non-error bootstrap rejections", async () => {
    const { root, flush, captureCharFrame, waitFor } = await setupTest();

    root.render(
      <App
        mode="admin"
        client={fakeClient(() => Promise.reject("offline"))}
      />,
    );
    await flush();

    await waitFor(async () => {
      const frame = captureCharFrame();
      return frame.includes("offline");
    });
  });
});

function fakeClient(
  requestOverride?: (
    method: string,
    params: Record<string, unknown>,
  ) => Promise<Record<string, unknown>>,
): RpcClient {
  return {
    request: (method: string, params: Record<string, unknown>) => {
      if (requestOverride) {
        return requestOverride(method, params);
      }
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
          "Dream Audits",
          "Audit Log",
        ],
        default_view: "Crystals",
        header: {
          product: "Hieronymus",
          version: "0.1.0",
          tagline: "Local translation memory.",
          logo: {
            text: "H",
            name: "feather",
            alt: "Hieronymus feather logo",
          },
        },
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
        short_term_status: {
          pending_count: 0,
          min_pending_short_term_memories: 20,
          max_pending_short_term_memories: 200,
          urgent: false,
          drain_in_progress: false,
          drain_completed: 0,
          drain_remaining: 0,
          drain_total: 0,
          drain_progress: 0,
        },
        dream_status: {
          state: "DISABLED",
          current_phase: "",
          progress: 0,
          run_id: null,
          cycle_id: null,
          owner: "",
          started_at: "",
        },
        config_editor: {
          config: {},
          config_error: "",
          providers: {
            anthropic: {
              provider_type: "anthropic",
              model: "claude-sonnet-4-20250514",
            },
          },
          workflows: {
            crystallization: {
              provider: "anthropic",
              model: "claude-sonnet-4-20250514",
            },
          },
          prompts: {
            general: "Translate with continuity.",
          },
          thresholds: {
            min_pending_short_term_memories: 20,
            max_pending_short_term_memories: 200,
            max_short_term_memories_per_cycle: 50,
          },
          model_cache: {},
          model_cache_warnings: [],
        },
      });
    },
    close: () => {},
  };
}
