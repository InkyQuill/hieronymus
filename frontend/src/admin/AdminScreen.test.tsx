import React from "react";
import { act } from "react";
import { describe, expect, it } from "vitest";
import { testRender } from "@opentui/react/test-utils";
import type { RpcClient } from "../rpc/client.js";
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
      model_cache_warnings: [
        {
          workflow: "crystallization",
          provider: "anthropic",
          code: "model_cache_missing",
          message: "model cache has not been fetched for provider",
        },
      ],
    },
  };
}

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
  const input = {
    type: async (value: string) => {
      const current = await ensureSetup();
      for (const key of value) {
        act(() => {
          current.mockInput.pressKey(key);
        });
      }
      await flush();
    },
  };
  return {
    root: {
      render: (next: React.ReactNode) => {
        node = next;
      },
    },
    mockInput: input,
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

describe("AdminScreen", () => {
  it("renders views, stats, table row, and detail", async () => {
    const { root, flush, captureCharFrame } = await setupTest();

    root.render(<AdminScreen initial={bootstrap()} client={undefined} />);
    await flush();

    const output = captureCharFrame();
    expect(output).toContain("Crystals");
    expect(output).toContain("H Hieronymus Admin 0.1.0");
    expect(output).toContain("Local translation memory.");
    expect(output).toContain("series 1");
    expect(output).toContain("Short-term pending 0");
    expect(output).toContain("Dream DISABLED");
    expect(output).toContain("Config providers anthropic");
    expect(output).toContain(
      "workflows crystallization:claude-sonnet-4-20250514",
    );
    expect(output).toContain("model cache warnings 1");
    expect(output).toContain("model cache has not been fetched for provider");
    expect(output).toContain("Guild Ledger");
    expect(output).toContain("Guild ledger detail marker.");
  });

  it("shows crystal commands for crystal view", async () => {
    const { root, flush, captureCharFrame } = await setupTest();

    root.render(
      <AdminScreen initial={bootstrap()} client={undefined} showCommands />,
    );
    await flush();

    const output = captureCharFrame();
    expect(output).toContain("reinforce");
    expect(output).toContain("delete");
    expect(output).not.toContain("approve");
  });

  it("handles filter and edit keys as placeholder commands", async () => {
    const client = fakeClient(() =>
      Promise.reject(new Error("unexpected request")),
    );
    const { root, mockInput, flush, captureCharFrame, waitFor } =
      await setupTest();

    root.render(<AdminScreen initial={bootstrap()} client={client} />);
    await flush();

    const output = captureCharFrame();
    expect(output).toContain("[f] filter");
    expect(output).toContain("[e] edit");
    expect(output).toContain("[1-9] view");

    await mockInput.type("f");
    await waitFor(async () => {
      const frame = captureCharFrame();
      return frame.includes("Filter command selected");
    });

    await mockInput.type("e");
    await waitFor(async () => {
      const frame = captureCharFrame();
      return frame.includes("Edit Memory");
    });
  });

  it("reinforces the selected crystal and refreshes from nested snapshot", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve({
        result: { ok: true },
        stats: {
          series: 2,
          crystals: 1,
          lessons: 0,
          short_term_memories: 0,
          sessions: 0,
          dream_runs: 0,
          pending_proposals: 0,
          audit_events: 0,
        },
        snapshot: {
          ...bootstrap().snapshot,
          rows: [
            {
              ...bootstrap().snapshot.rows[0],
              quality_label: "reinforced",
            },
          ],
          selected: {
            ...bootstrap().snapshot.selected!,
            quality_label: "reinforced",
          },
          detail: {
            ...bootstrap().snapshot.detail,
            body: "Reinforced detail marker.",
          },
        },
        short_term_status: {
          ...bootstrap().short_term_status,
          pending_count: 3,
          drain_in_progress: true,
          drain_completed: 7,
          drain_remaining: 3,
          drain_total: 10,
          drain_progress: 0.7,
        },
        dream_status: {
          ...bootstrap().dream_status,
          state: "WORKING",
          current_phase: "maintenance",
          progress: 0.75,
          run_id: 12,
          cycle_id: 4,
        },
      });
    });
    const { root, mockInput, flush, captureCharFrame, waitFor } =
      await setupTest();

    root.render(<AdminScreen initial={bootstrap()} client={client} />);
    await flush();

    await mockInput.type("+");

    await waitFor(async () => {
      const frame = captureCharFrame();
      return frame.includes("Reinforced detail marker.");
    });

    expect(calls).toEqual([
      {
        method: "admin.reinforce_crystal",
        params: { id: 1, view: "Crystals" },
      },
    ]);

    const output = captureCharFrame();
    expect(output).toContain("series 2");
    expect(output).toContain("reinforced");
    expect(output).toContain("Short-term pending 3");
    expect(output).toContain("drain 7/10 (70%) remaining 3");
    expect(output).toContain("Dream WORKING");
    expect(output).toContain("phase maintenance");
    expect(output).toContain("progress 75%");
  });

  it("does not send crystal mutations from proposals", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.reject(new Error("unexpected mutation"));
    });
    const { root, mockInput, flush } = await setupTest();

    root.render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: snapshotForView("Proposals"),
        }}
        client={client}
      />,
    );
    await flush();

    await mockInput.type("d");
    await mockInput.type("+");
    await flush();

    expect(calls).toEqual([]);
  });

  it("reinforces lessons with the current view and preserves lessons", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve({
        stats: bootstrap().stats,
        snapshot: {
          ...snapshotForView("Lessons"),
          detail: {
            ...bootstrap().snapshot.detail,
            body: "Lesson reinforcement marker.",
          },
        },
      });
    });
    const { root, mockInput, flush, captureCharFrame, waitFor } =
      await setupTest();

    root.render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: snapshotForView("Lessons"),
        }}
        client={client}
      />,
    );
    await flush();

    await mockInput.type("+");

    await waitFor(async () => {
      const frame = captureCharFrame();
      return frame.includes("Lesson reinforcement marker.");
    });

    expect(calls).toEqual([
      {
        method: "admin.reinforce_crystal",
        params: { id: 1, view: "Lessons" },
      },
    ]);

    expect(captureCharFrame()).toContain("> Lessons");
  });

  it("navigates to eighth and ninth backend views", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve({
        stats: bootstrap().stats,
        snapshot: snapshotForView(String(params.view)),
      });
    });
    const { root, mockInput, flush, captureCharFrame, waitFor } =
      await setupTest();

    root.render(<AdminScreen initial={bootstrap()} client={client} />);
    await flush();

    await mockInput.type("8");

    await waitFor(async () => {
      const frame = captureCharFrame();
      return frame.includes("Loaded Dream Audits");
    });

    await mockInput.type("9");

    await waitFor(async () => {
      const frame = captureCharFrame();
      return frame.includes("Loaded Audit Log");
    });

    expect(calls).toEqual([
      {
        method: "admin.snapshot",
        params: { view: "Dream Audits" },
      },
      {
        method: "admin.snapshot",
        params: { view: "Audit Log" },
      },
    ]);

    expect(captureCharFrame()).toContain("> Audit Log");
  });

  it("ignores view and action keys while an operation is in flight", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const deferred = deferredResponse();
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return deferred.promise;
    });
    const { root, mockInput, flush, captureCharFrame, waitFor } =
      await setupTest();

    root.render(<AdminScreen initial={bootstrap()} client={client} />);
    await flush();

    await mockInput.type("1");
    await mockInput.type("2");
    await mockInput.type("+");

    await waitFor(async () => calls.length >= 1);

    expect(calls).toEqual([
      {
        method: "admin.snapshot",
        params: { view: "Concepts" },
      },
    ]);

    deferred.resolve({
      stats: bootstrap().stats,
      snapshot: snapshotForView("Concepts"),
    });

    await waitFor(async () => {
      const frame = captureCharFrame();
      return frame.includes("Loaded Concepts");
    });
  });
});

function snapshotForView(view: string) {
  return {
    ...bootstrap().snapshot,
    view,
    detail: {
      ...bootstrap().snapshot.detail,
      subtitle: view,
    },
  };
}

function fakeClient(
  request: (
    method: string,
    params: Record<string, unknown>,
  ) => Promise<Record<string, unknown>>,
): RpcClient {
  return { request, close: () => {} };
}

function deferredResponse() {
  let resolve!: (payload: Record<string, unknown>) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<Record<string, unknown>>(
    (promiseResolve, promiseReject) => {
      resolve = promiseResolve;
      reject = promiseReject;
    },
  );
  return { promise, resolve, reject };
}
