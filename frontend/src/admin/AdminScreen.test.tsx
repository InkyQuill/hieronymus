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
