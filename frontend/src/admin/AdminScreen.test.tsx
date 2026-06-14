import { afterEach, describe, expect, it } from "bun:test";
import type { RpcClient } from "../rpc/client.js";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { AdminScreen } from "./AdminScreen.js";
import { CommandPalette, commandsForView } from "./CommandPalette.js";
import { HelpOverlay } from "./HelpOverlay.js";

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
    command_options: [
      {
        id: "add_memory",
        label: "Add Memory",
        hint: "Create a new crystal in the current memory view.",
        key: "a",
        group: "Memory",
        views: ["Crystals", "Lessons"],
        requires_selection: false,
      },
      {
        id: "edit_memory",
        label: "Edit Memory",
        hint: "Edit the selected crystal or lesson text.",
        key: "e",
        group: "Memory",
        views: ["Crystals", "Lessons"],
        requires_selection: true,
      },
      {
        id: "reinforce_crystal",
        label: "Reinforce Crystal",
        hint: "Increase strength/confidence for the selected crystal or lesson.",
        key: "+",
        group: "Memory",
        views: ["Crystals", "Lessons"],
        requires_selection: true,
      },
      {
        id: "decay_crystal",
        label: "Decay Crystal",
        hint: "Decrease strength/confidence for the selected crystal or lesson.",
        key: "-",
        group: "Memory",
        views: ["Crystals", "Lessons"],
        requires_selection: true,
      },
      {
        id: "inspect_provenance",
        label: "Inspect Provenance",
        hint: "Load provenance data for the selected crystal or lesson.",
        key: "p",
        group: "Inspect",
        views: ["Crystals", "Lessons"],
        requires_selection: true,
      },
      {
        id: "inspect_recall_reasons",
        label: "Inspect Recall Reasons",
        hint: "Load recall reason data for the selected crystal or lesson.",
        key: "r",
        group: "Inspect",
        views: ["Crystals", "Lessons"],
        requires_selection: true,
      },
      {
        id: "approve_proposal",
        label: "Approve Proposal",
        hint: "Approve the selected compatibility proposal.",
        key: "a",
        group: "Proposals",
        views: ["Proposals"],
        requires_selection: true,
      },
      {
        id: "run_manual_dreaming",
        label: "Run Manual Dreaming",
        hint: "Run dreaming manually and select the resulting dream run.",
        key: "D",
        group: "Dreaming",
        views: ["Dream Runs"],
        requires_selection: false,
      },
      {
        id: "review_dream_output",
        label: "Review Dream Output",
        hint: "Load the review payload for the selected dream run.",
        key: "enter",
        group: "Dreaming",
        views: ["Dream Runs"],
        requires_selection: true,
      },
    ],
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

function setupTest() {
  return createOpenTuiHarness({ width: 160, height: 60 });
}

function setupSizedTest(width: number, height: number) {
  return createOpenTuiHarness({ width, height });
}

function longestFrameLine(output: string) {
  return Math.max(...output.split("\n").map((line) => line.trimEnd().length));
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("AdminScreen", () => {
  it("renders admin as a single active pane at 80x24", async () => {
    const { render, waitForFrame } = setupSizedTest(80, 24);

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);

    const output = await waitForFrame((frame) => frame.includes("Views"));
    expect(output).toContain("H Hieronymus Admin 0.1.0");
    expect(output).toContain("Views");
    expect(output).toContain("Crystals");
    expect(output).toContain("Tab pane");
    expect(output).not.toContain("Detail Inspector");
  });

  it("cycles compact admin panes with tab", async () => {
    const { render, mockInput, waitForFrame } = setupSizedTest(80, 24);

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);
    await mockInput.press("tab");

    const tableOutput = await waitForFrame((frame) =>
      frame.includes("Guild Ledger"),
    );
    expect(tableOutput).toContain("Crystals");
    expect(tableOutput).toContain("Guild Ledger");

    await mockInput.press("tab");
    const detailOutput = await waitForFrame((frame) =>
      frame.includes("Guild ledger detail marker."),
    );
    expect(detailOutput).toContain("Detail Inspector");
    expect(detailOutput).toContain("Guild ledger detail marker.");
  });

  it("renders command palette in compact admin layout", async () => {
    const { render, mockInput, waitForFrame } = setupSizedTest(80, 24);

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);
    await mockInput.press("p", { ctrl: true });

    const output = await waitForFrame((frame) =>
      frame.includes("Command Palette"),
    );
    expect(output).toContain("Command Palette");
    expect(output).toContain("Enter run Esc close");
  });

  it("renders help in compact admin layout", async () => {
    const { render, mockInput, waitForFrame } = setupSizedTest(80, 24);

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);
    await mockInput.type("?");

    const output = await waitForFrame((frame) => frame.includes("Help"));
    expect(output).toContain("Help");
    expect(output).toContain("Esc/? close");
  });

  it("keeps command palette inside the narrow admin viewport", async () => {
    const { render, mockInput, waitForFrame } = setupSizedTest(60, 20);

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);
    await mockInput.press("p", { ctrl: true });

    const output = await waitForFrame((frame) =>
      frame.includes("Command Palette"),
    );
    expect(output).toContain("Command Palette");
    expect(output).toContain("Enter run Esc close");
    expect(output).not.toContain("Terminal too small");
    expect(longestFrameLine(output)).toBeLessThanOrEqual(60);
  });

  it("keeps help inside the narrow admin viewport", async () => {
    const { render, mockInput, waitForFrame } = setupSizedTest(60, 20);

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);
    await mockInput.type("?");

    const output = await waitForFrame((frame) => frame.includes("Help"));
    expect(output).toContain("Help");
    expect(output).toContain("Esc/? close");
    expect(output).not.toContain("Terminal too small");
    expect(longestFrameLine(output)).toBeLessThanOrEqual(60);
  });

  it("honors compact command palette width", async () => {
    const data = bootstrap();
    const { render, waitForFrame } = setupSizedTest(80, 24);

    await render(
      <CommandPalette
        commands={commandsForView(
          data.command_options,
          data.snapshot.view,
          Boolean(data.snapshot.selected),
        )}
        selectedIndex={0}
        width={46}
      />,
    );

    const output = await waitForFrame((frame) =>
      frame.includes("Command Palette"),
    );
    expect(longestFrameLine(output)).toBeLessThanOrEqual(46);
  });

  it("honors compact help overlay width", async () => {
    const data = bootstrap();
    const { render, waitForFrame } = setupSizedTest(80, 24);

    await render(
      <HelpOverlay
        commands={data.command_options}
        view={data.snapshot.view}
        width={46}
      />,
    );

    const output = await waitForFrame((frame) => frame.includes("Help"));
    expect(longestFrameLine(output)).toBeLessThanOrEqual(46);
  });

  it("renders normal admin content at the minimum terminal size", async () => {
    const { render, waitForFrame } = setupSizedTest(60, 20);

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);

    const output = await waitForFrame((frame) => frame.includes("Views"));
    expect(output).toContain("Views");
    expect(output).toContain("Crystals");
    expect(output).not.toContain("Terminal too small");
  });

  it("renders a too-small admin message below the minimum width", async () => {
    const { render, waitForFrame } = setupSizedTest(59, 20);

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);

    const output = await waitForFrame((frame) =>
      frame.includes("Terminal too small"),
    );
    expect(output).toContain("59x20");
    expect(output).toContain("minimum 60x20");
  });

  it("does not open hidden dialogs below the minimum terminal size", async () => {
    const { render, mockInput, waitForFrame } = setupSizedTest(59, 20);

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);
    await mockInput.type("a");

    const output = await waitForFrame((frame) =>
      frame.includes("Terminal too small"),
    );
    expect(output).toContain("Terminal too small");
    expect(output).not.toContain("Add New Crystal / Lesson / Rule");
  });

  it("renders views, stats, table row, and detail", async () => {
    const { render, waitForFrame } = setupTest();

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);

    const output = await waitForFrame((frame) => frame.includes("Crystals"));
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

  it("opens a keyboard command palette with context commands", async () => {
    const { render, waitForFrame } = setupTest();

    await render(
      <AdminScreen initial={bootstrap()} client={undefined} showCommands />,
    );

    const output = await waitForFrame((frame) =>
      frame.includes("Command Palette"),
    );
    expect(output).toContain("Command Palette");
    expect(output).toContain("> Add Memory [a]");
    expect(output).toContain("Reinforce Crystal [+]");
    expect(output).toContain("Inspect Recall Reasons [r]");
    expect(output).toContain("Enter run Esc close");
    expect(output).not.toContain("Approve Proposal");
  });

  it("opens contextual help with question mark and closes with escape", async () => {
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);

    await mockInput.type("?");

    let output = await waitForFrame((frame) => frame.includes("Help"));
    expect(output).toContain("Esc/? close");
    expect(output).toContain("Ctrl+P commands");
    expect(output).not.toContain("q quit");
    expect(output).toContain("[+] Reinforce Crystal");
    expect(output).not.toContain("Approve Proposal");

    await mockInput.press("escape");
    output = await waitForFrame((frame) => !frame.includes("Help"));
    expect(output).not.toContain("Help");
  });

  it("runs selected command palette actions through existing RPC handlers", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve({
        stats: bootstrap().stats,
        snapshot: {
          ...bootstrap().snapshot,
          detail: {
            ...bootstrap().snapshot.detail,
            body: "Palette reinforcement marker.",
          },
        },
      });
    });
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<AdminScreen initial={bootstrap()} client={client} />);

    await mockInput.press("p", { ctrl: true });
    await mockInput.press("j");
    await mockInput.press("j");
    await mockInput.press("enter");

    await waitForFrame((frame) =>
      frame.includes("Palette reinforcement marker."),
    );

    expect(calls).toEqual([
      {
        method: "admin.reinforce_crystal",
        params: { id: 1, view: "Crystals" },
      },
    ]);
  });

  it("keeps command palette modal over direct hotkeys", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.reject(new Error("unexpected request"));
    });
    const { render, mockInput, captureCharFrame } = setupTest();

    await render(<AdminScreen initial={bootstrap()} client={client} />);

    await mockInput.press("p", { ctrl: true });
    await mockInput.press("1");
    await mockInput.press("+");
    await mockInput.press("a");

    const output = captureCharFrame();
    expect(calls).toEqual([]);
    expect(output).toContain("Command Palette");
    expect(output).not.toContain("Add New Crystal / Lesson / Rule");
  });

  it("runs generic palette snapshot commands without rendered filter labels", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const dreamRunsSnapshot = {
      ...snapshotForView("Dream Runs"),
      filters: ["phase active"],
    };
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve({
        stats: bootstrap().stats,
        snapshot: snapshotForView("Dream Runs"),
      });
    });
    const { render, mockInput, waitFor } = setupTest();

    await render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: dreamRunsSnapshot,
        }}
        client={client}
      />,
    );

    await mockInput.press("p", { ctrl: true });
    await mockInput.press("enter");

    await waitFor(async () => calls.length >= 1);

    expect(calls).toEqual([
      {
        method: "admin.run_manual_dreaming",
        params: { view: "Dream Runs" },
      },
    ]);
  });

  it("navigates rows without sending rendered filter labels", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const base = bootstrap().snapshot;
    const filteredSnapshot = {
      ...base,
      filters: ["status=active"],
      rows: [
        base.rows[0],
        {
          ...base.rows[0],
          id: 2,
          label: "Second Crystal",
        },
      ],
    };
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve({
        stats: bootstrap().stats,
        snapshot: {
          ...filteredSnapshot,
          selected: filteredSnapshot.rows[1],
        },
      });
    });
    const { render, mockInput, waitFor } = setupTest();

    await render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: filteredSnapshot,
        }}
        client={client}
      />,
    );

    await mockInput.press("tab");
    await mockInput.press("down");

    await waitFor(async () => calls.length >= 1);

    expect(calls).toEqual([
      {
        method: "admin.snapshot",
        params: { view: "Crystals", selected_id: 2 },
      },
    ]);
  });

  it("navigates rows with j and k like arrow keys", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const base = bootstrap().snapshot;
    const rows = [
      base.rows[0],
      {
        ...base.rows[0],
        id: 2,
        label: "Second Crystal",
      },
    ];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      const selected = rows.find((row) => row.id === params.selected_id);
      return Promise.resolve({
        stats: bootstrap().stats,
        snapshot: {
          ...base,
          rows,
          selected,
          detail: {
            ...base.detail,
            title: selected?.label ?? base.detail.title,
            body: `${selected?.label ?? "Unknown"} detail marker.`,
          },
        },
      });
    });
    const { render, mockInput, waitForFrame } = setupTest();

    await render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: { ...base, rows, selected: rows[0] },
        }}
        client={client}
      />,
    );

    await mockInput.press("tab");
    await mockInput.type("j");
    await waitForFrame((frame) => frame.includes("Selected Second Crystal"));

    await mockInput.type("k");
    await waitForFrame((frame) => frame.includes("Selected Guild Ledger"));

    expect(calls).toEqual([
      {
        method: "admin.snapshot",
        params: { view: "Crystals", selected_id: 2 },
      },
      {
        method: "admin.snapshot",
        params: { view: "Crystals", selected_id: 1 },
      },
    ]);
  });

  it("navigates views with j and k like arrow keys", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve({
        stats: bootstrap().stats,
        snapshot: snapshotForView(String(params.view)),
      });
    });
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<AdminScreen initial={bootstrap()} client={client} />);

    await mockInput.type("j");
    await waitForFrame((frame) => frame.includes("Loaded Lessons"));

    await mockInput.type("k");
    await waitForFrame((frame) => frame.includes("Loaded Crystals"));

    expect(calls).toEqual([
      {
        method: "admin.snapshot",
        params: { view: "Lessons" },
      },
      {
        method: "admin.snapshot",
        params: { view: "Crystals" },
      },
    ]);
  });

  it("searches current rows and selects the first matching row", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const base = bootstrap().snapshot;
    const rows = [
      base.rows[0],
      {
        ...base.rows[0],
        id: 2,
        kind: "lesson",
        label: "Canal Registry",
        status: "active",
        scope: "library-wing",
        language_pair: "en -> ru",
        quality_label: "draft",
        tags: ["waterways"],
        summary: "Contains the lockkeeper oath.",
      },
    ];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      const selected = rows.find((row) => row.id === params.selected_id);
      return Promise.resolve({
        stats: bootstrap().stats,
        snapshot: {
          ...base,
          rows,
          selected,
          detail: {
            ...base.detail,
            title: selected?.label ?? base.detail.title,
            body: `${selected?.label ?? "Unknown"} detail marker.`,
          },
        },
      });
    });
    const { render, mockInput, waitForFrame } = setupTest();

    await render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: { ...base, rows, selected: rows[0] },
        }}
        client={client}
      />,
    );

    await mockInput.press("tab");
    await mockInput.type("/");
    await waitForFrame((frame) => frame.includes("Search: "));

    await mockInput.type("lockkeeper");
    await waitForFrame((frame) => frame.includes("Search: lockkeeper"));

    await mockInput.press("enter");

    const output = await waitForFrame((frame) =>
      frame.includes("Found Canal Registry"),
    );

    expect(calls).toEqual([
      {
        method: "admin.snapshot",
        params: { view: "Crystals", selected_id: 2 },
      },
    ]);
    expect(output).toContain("Canal Registry detail marker.");
  });

  it("keeps question marks inside admin search mode", async () => {
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);

    await mockInput.type("/");
    await mockInput.type("?");

    const output = await waitForFrame((frame) => frame.includes("Search: ?"));
    expect(output).toContain("Search: ?");
    expect(output).not.toContain("Help");
  });

  it("shows mode-aware compact admin search footer", async () => {
    const { render, mockInput, waitForFrame } = setupSizedTest(80, 24);

    await render(<AdminScreen initial={bootstrap()} client={undefined} />);

    await mockInput.type("/");

    const output = await waitForFrame((frame) => frame.includes("Enter search"));
    expect(output).toContain("Enter search");
    expect(output).toContain("Esc cancel");
    expect(output).not.toContain("q quit");
  });

  it("runs visible proposal command shortcuts directly", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve({
        stats: bootstrap().stats,
        snapshot: snapshotForView("Proposals"),
      });
    });
    const { render, mockInput, waitForFrame } = setupTest();

    await render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: snapshotForView("Proposals"),
        }}
        client={client}
      />,
    );

    await mockInput.type("a");

    const output = await waitForFrame((frame) =>
      frame.includes("Approved proposal"),
    );
    expect(output).not.toContain("Add New Crystal");
    expect(calls).toEqual([
      {
        method: "admin.approve_proposal",
        params: { id: 1, view: "Proposals" },
      },
    ]);
  });

  it("runs visible dream command shortcuts directly", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      if (method === "admin.dream_review") {
        return Promise.resolve({
          stats: bootstrap().stats,
          snapshot: snapshotForView("Dream Runs"),
          review: {
            consumed_memories: "Direct review memory marker.",
            created_crystals: ["Direct review crystal marker."],
            run_id: 1,
            failed_outputs: [],
            validation_errors: [],
          },
        });
      }
      return Promise.resolve({
        stats: bootstrap().stats,
        snapshot: snapshotForView("Dream Runs"),
      });
    });
    const { render, mockInput, waitForFrame } = setupTest();

    await render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: snapshotForView("Dream Runs"),
        }}
        client={client}
      />,
    );

    await mockInput.type("D");
    await waitForFrame((frame) => frame.includes("Ran manual dreaming"));

    await mockInput.press("enter");
    const output = await waitForFrame((frame) =>
      frame.includes("Direct review memory marker."),
    );

    expect(output).toContain("Direct review crystal marker.");
    expect(calls).toEqual([
      {
        method: "admin.run_manual_dreaming",
        params: { view: "Dream Runs" },
      },
      {
        method: "admin.dream_review",
        params: { id: 1, view: "Dream Runs" },
      },
    ]);
  });

  it("shows dream review payload from the palette command", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.resolve({
        stats: bootstrap().stats,
        snapshot: {
          ...snapshotForView("Dream Runs"),
          detail: {
            title: "Dream run",
            subtitle: "Dream Runs",
            body: "Stale refreshed dream detail.",
            fields: [],
          },
        },
        review: {
          consumed_memories: "Review payload memory marker.",
          created_crystals: ["Review payload crystal marker."],
          run_id: 1,
          failed_outputs: [],
          validation_errors: [],
        },
      });
    });
    const { render, mockInput, waitForFrame } = setupTest();

    await render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: snapshotForView("Dream Runs"),
        }}
        client={client}
      />,
    );

    await mockInput.press("p", { ctrl: true });
    await mockInput.press("j");
    await mockInput.press("enter");

    const output = await waitForFrame(
      (frame) =>
        frame.includes("Review payload memory marker.") &&
        frame.includes("Review payload crystal marker."),
    );

    expect(output).toContain("admin.dream_review");
    expect(output).toContain("Review payload memory marker.");
    expect(output).toContain("Review payload crystal marker.");
    expect(output).not.toContain("Stale refreshed dream detail.");
    expect(calls).toEqual([
      {
        method: "admin.dream_review",
        params: { id: 1, view: "Dream Runs" },
      },
    ]);
  });

  it("handles filter and edit keys as placeholder commands", async () => {
    const client = fakeClient(() =>
      Promise.reject(new Error("unexpected request")),
    );
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<AdminScreen initial={bootstrap()} client={client} />);

    const output = await waitForFrame((frame) =>
      frame.includes("[Ctrl+P] commands"),
    );
    expect(output).toContain("[Ctrl+P] commands");
    expect(output).toContain("[?] help");
    expect(output).toContain("[1-9] view");

    await mockInput.type("f");
    await waitForFrame((frame) => frame.includes("Filter command selected"));

    await mockInput.type("e");
    await waitForFrame((frame) => frame.includes("Edit Memory"));
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
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<AdminScreen initial={bootstrap()} client={client} />);

    await mockInput.type("+");

    const output = await waitForFrame((frame) =>
      frame.includes("Reinforced detail marker."),
    );

    expect(calls).toEqual([
      {
        method: "admin.reinforce_crystal",
        params: { id: 1, view: "Crystals" },
      },
    ]);

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
    const { render, mockInput } = setupTest();

    await render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: snapshotForView("Proposals"),
        }}
        client={client}
      />,
    );

    await mockInput.type("d");
    await mockInput.type("+");

    expect(calls).toEqual([]);
  });

  it("does not open delete for short-term session rows", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return Promise.reject(new Error("unexpected request"));
    });
    const { render, mockInput, waitForFrame } = setupTest();

    await render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: snapshotForView("Short-Term Sessions"),
        }}
        client={client}
      />,
    );

    await mockInput.type("d");

    const frame = await waitForFrame((output) =>
      output.includes("Delete not supported for this view"),
    );

    expect(frame).not.toContain("Confirm");
    expect(calls).toEqual([]);
  });

  it("keeps multiline memory editor bounded in the admin dialog", async () => {
    const longBody = Array.from(
      { length: 20 },
      (_, index) => `Long memory line ${index + 1}`,
    ).join("\n");
    const { render, mockInput, waitForFrame } = setupSizedTest(80, 24);

    await render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: {
            ...bootstrap().snapshot,
            detail: {
              ...bootstrap().snapshot.detail,
              body: longBody,
            },
          },
        }}
        client={undefined}
      />,
    );

    await mockInput.type("e");

    const output = await waitForFrame((frame) => frame.includes("Edit Memory"));
    expect(output).toContain("Edit Memory");
    expect(output).toContain("Text:");
    expect(output).toContain("[Enter] Submit");
    expect(longestFrameLine(output)).toBeLessThanOrEqual(80);
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
    const { render, mockInput, waitForFrame } = setupTest();

    await render(
      <AdminScreen
        initial={{
          ...bootstrap(),
          snapshot: snapshotForView("Lessons"),
        }}
        client={client}
      />,
    );

    await mockInput.type("+");

    const output = await waitForFrame((frame) =>
      frame.includes("Lesson reinforcement marker."),
    );

    expect(calls).toEqual([
      {
        method: "admin.reinforce_crystal",
        params: { id: 1, view: "Lessons" },
      },
    ]);

    expect(output).toContain("> Lessons");
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
    const { render, mockInput, waitForFrame } = setupTest();

    await render(<AdminScreen initial={bootstrap()} client={client} />);

    await mockInput.type("8");

    await waitForFrame((frame) => frame.includes("Loaded Dream Audits"));

    await mockInput.type("9");

    const output = await waitForFrame((frame) =>
      frame.includes("Loaded Audit Log"),
    );

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

    expect(output).toContain("> Audit Log");
  });

  it("ignores view and action keys while an operation is in flight", async () => {
    const calls: Array<{ method: string; params: Record<string, unknown> }> =
      [];
    const deferred = deferredResponse();
    const client = fakeClient((method, params) => {
      calls.push({ method, params });
      return deferred.promise;
    });
    const { render, mockInput, waitFor, waitForFrame } = setupTest();

    await render(<AdminScreen initial={bootstrap()} client={client} />);

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

    await waitForFrame((frame) => frame.includes("Loaded Concepts"));
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
