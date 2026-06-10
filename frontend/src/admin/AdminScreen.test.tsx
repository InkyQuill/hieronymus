import React from "react";
import { describe, expect, it } from "vitest";
import { render } from "ink-testing-library";
import type { JsonRpcClient } from "../rpc/client.js";
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

describe("AdminScreen", () => {
  it("renders views, stats, table row, and detail", () => {
    const app = render(
      <AdminScreen initial={bootstrap()} client={undefined} />,
    );

    expect(app.lastFrame()).toContain("Crystals");
    expect(app.lastFrame()).toContain("H Hieronymus Admin 0.1.0");
    expect(app.lastFrame()).toContain("Local translation memory.");
    expect(app.lastFrame()).toContain("series 1");
    expect(app.lastFrame()).toContain("Short-term pending 0");
    expect(app.lastFrame()).toContain("Dream DISABLED");
    expect(app.lastFrame()).toContain("Config providers anthropic");
    expect(app.lastFrame()).toContain(
      "workflows crystallization:claude-sonnet-4-20250514",
    );
    expect(app.lastFrame()).toContain("model cache warnings 1");
    expect(app.lastFrame()).toContain(
      "model cache has not been fetched for provider",
    );
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

  it("handles filter and edit keys as placeholder commands", async () => {
    const client = fakeClient(() =>
      Promise.reject(new Error("unexpected request")),
    );
    const app = render(<AdminScreen initial={bootstrap()} client={client} />);

    expect(app.lastFrame()).toContain("f filter");
    expect(app.lastFrame()).toContain("e edit");
    expect(app.lastFrame()).toContain("1-9 view");

    await nextTick();
    app.stdin.write("f");
    await waitFor(() =>
      expect(app.lastFrame()).toContain("Filter command selected"),
    );

    app.stdin.write("e");
    await waitFor(() =>
      expect(app.lastFrame()).toContain("Edit command selected"),
    );
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
    const app = render(<AdminScreen initial={bootstrap()} client={client} />);

    await nextTick();
    app.stdin.write("+");
    await waitFor(() =>
      expect(app.lastFrame()).toContain("Reinforced detail marker."),
    );

    expect(calls).toEqual([
      {
        method: "admin.reinforce_crystal",
        params: { id: 1, view: "Crystals" },
      },
    ]);
    expect(app.lastFrame()).toContain("series 2");
    expect(app.lastFrame()).toContain("reinforced");
    expect(app.lastFrame()).toContain("Short-term pending 3");
    expect(app.lastFrame()).toContain("drain 7/10 (70%) remaining 3");
    expect(app.lastFrame()).toContain("Dream WORKING");
    expect(app.lastFrame()).toContain("phase maintenance");
    expect(app.lastFrame()).toContain("progress 75%");
  });

  it("does not send crystal mutations from proposals", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.reject(new Error("unexpected mutation"));
    });
    const app = render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: snapshotForView("Proposals"),
        }}
        client={client}
      />,
    );

    await nextTick();
    app.stdin.write("d");
    app.stdin.write("+");
    await nextTick();

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
    const app = render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: snapshotForView("Lessons"),
        }}
        client={client}
      />,
    );

    await nextTick();
    app.stdin.write("+");
    await waitFor(() =>
      expect(app.lastFrame()).toContain("Lesson reinforcement marker."),
    );

    expect(calls).toEqual([
      {
        method: "admin.reinforce_crystal",
        params: { id: 1, view: "Lessons" },
      },
    ]);
    expect(app.lastFrame()).toContain("> Lessons");
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
    const app = render(<AdminScreen initial={bootstrap()} client={client} />);

    await nextTick();
    app.stdin.write("8");
    await waitFor(() =>
      expect(app.lastFrame()).toContain("Loaded Dream Audits"),
    );

    app.stdin.write("9");
    await waitFor(() => expect(app.lastFrame()).toContain("Loaded Audit Log"));

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
    expect(app.lastFrame()).toContain("> Audit Log");
  });

  it("ignores view and action keys while an operation is in flight", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const deferred = deferredResponse();
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return deferred.promise;
    });
    const app = render(<AdminScreen initial={bootstrap()} client={client} />);

    await nextTick();
    app.stdin.write("1");
    app.stdin.write("2");
    app.stdin.write("+");
    await waitFor(() => expect(calls).toHaveLength(1));

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
    await waitFor(() => expect(app.lastFrame()).toContain("Loaded Concepts"));
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
): JsonRpcClient {
  return { request } as unknown as JsonRpcClient;
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

async function nextTick() {
  await new Promise((resolve) => setTimeout(resolve, 0));
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
