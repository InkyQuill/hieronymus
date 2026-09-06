import { render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import { loadAdminSnapshot, runAdminAction } from "../lib/api";
import type {
  AdminActionResult,
  AdminDashboard,
  AdminSnapshot,
} from "../lib/types";
import MemoryViews from "./MemoryViews.svelte";

vi.mock("../lib/api", () => ({
  loadAdminSnapshot: vi.fn(),
  runAdminAction: vi.fn(),
}));

const loadSnapshotMock = vi.mocked(loadAdminSnapshot);
const runActionMock = vi.mocked(runAdminAction);

const command = (
  id: string,
  label: string,
  views: string[],
  requires_selection: boolean,
) => ({
  id,
  label,
  hint: `${label} hint`,
  key: "",
  group: "Memory",
  views,
  requires_selection,
});

const dashboard = {
  header: {
    product: "Hieronymus",
    version: "0.4.0",
    tagline: "Translation memory",
  },
  stats: {},
  views: ["Crystals"],
  command_options: [
    command(
      "reinforce_crystal",
      "Reinforce Crystal",
      ["Crystals", "Lessons"],
      true,
    ),
    command("decay_crystal", "Decay Crystal", ["Crystals", "Lessons"], true),
    command(
      "delete_selected",
      "Delete Selected",
      ["Concepts", "Crystals", "Lessons"],
      true,
    ),
    command("split_crystal", "Split Crystal", ["Crystals", "Lessons"], true),
    command("add_memory", "Add Memory", ["Crystals", "Lessons"], false),
  ],
  short_term_status: {},
  dream_status: {},
} satisfies AdminDashboard;

const row = {
  id: 7,
  kind: "crystal",
  label: "Crystal Alpha",
  status: "active",
  scope: "series",
  language_pair: "en-ru",
  quality_label: "strong",
  tags: [],
};

const listSnapshot = {
  snapshot: {
    view: "Crystals",
    rows: [row],
    selected: null,
    detail: {
      title: "Crystals",
      subtitle: "Choose a crystal",
      body: "",
      fields: [],
    },
  },
} satisfies AdminSnapshot;

const selectedSnapshot = {
  snapshot: {
    view: "Crystals",
    rows: [row],
    selected: row,
    detail: {
      title: "Crystal Alpha",
      subtitle: "Selected",
      body: "Evidence",
      fields: [],
    },
  },
} satisfies AdminSnapshot;

beforeEach(() => {
  loadSnapshotMock.mockReset();
  runActionMock.mockReset();
  loadSnapshotMock
    .mockResolvedValueOnce(listSnapshot)
    .mockResolvedValue(selectedSnapshot);
});

test.each(["{Enter}", " "])(
  "memory rows load by keyboard activation: %s",
  async (key) => {
    const user = userEvent.setup();
    render(MemoryViews, { props: { dashboard, onNotice: vi.fn() } });
    const memoryRow = await screen.findByRole("button", {
      name: /Crystal Alpha/,
    });
    memoryRow.focus();
    await user.keyboard(key);
    await waitFor(() =>
      expect(loadSnapshotMock).toHaveBeenCalledWith("Crystals", 7),
    );
  },
);

const ALL_VIEWS = [
  "Concepts",
  "Renderings",
  "Crystals",
  "Lessons",
  "Short-Term Memory",
  "Short-Term Sessions",
  "Dream Runs",
  "Proposals",
  "Dream Audits",
  "Audit Log",
];

function snapshotFor(view: string): AdminSnapshot {
  const viewRow = {
    id: `${view}-1`,
    kind: `${view} kind`,
    label: `${view} record one`,
    status: "active",
    scope: "series:main",
    language_pair: "ja -> en",
    quality_label: "80% conf",
    tags: [],
  };
  return {
    snapshot: {
      view,
      rows: [viewRow],
      selected: viewRow,
      detail: {
        title: `${view} detail heading`,
        subtitle: `${view} detail`,
        body: `${view} body`,
        fields: [["Scope", "series:main"]],
      },
      filters: [],
    },
  } satisfies AdminSnapshot;
}

test("every advertised view is selectable and renders its returned rows", async () => {
  const user = userEvent.setup();
  loadSnapshotMock.mockReset();
  loadSnapshotMock.mockImplementation(async (view: string) =>
    snapshotFor(view),
  );

  const tenViewDashboard = {
    ...dashboard,
    views: ALL_VIEWS,
  } satisfies AdminDashboard;
  render(MemoryViews, {
    props: { dashboard: tenViewDashboard, onNotice: vi.fn() },
  });

  for (const view of ALL_VIEWS) {
    await user.click(screen.getByRole("button", { name: view }));
    await waitFor(() =>
      expect(loadSnapshotMock).toHaveBeenCalledWith(view, undefined),
    );
    await screen.findByText(`${view} record one`);
    await screen.findByText(`${view} body`);
  }
});

test("selection survives a background dashboard refresh by stable id", async () => {
  const user = userEvent.setup();
  loadSnapshotMock.mockReset();
  loadSnapshotMock
    .mockResolvedValueOnce(listSnapshot)
    .mockResolvedValue(selectedSnapshot);

  const { rerender } = render(MemoryViews, {
    props: { dashboard, onNotice: vi.fn() },
  });
  await user.click(
    await screen.findByRole("button", { name: /Crystal Alpha/ }),
  );
  await screen.findByText("Evidence");
  expect(loadSnapshotMock).toHaveBeenLastCalledWith("Crystals", 7);

  // A fresh dashboard object (what App.svelte hands down after an admin event)
  // must trigger a reload that keeps the selected row.
  await rerender({ dashboard: { ...dashboard }, onNotice: vi.fn() });
  await waitFor(() =>
    expect(loadSnapshotMock).toHaveBeenLastCalledWith("Crystals", 7),
  );
});

test("the dashboard effect does not fetch on first render", async () => {
  loadSnapshotMock.mockReset();
  loadSnapshotMock.mockResolvedValue(selectedSnapshot);
  render(MemoryViews, { props: { dashboard, onNotice: vi.fn() } });
  await waitFor(() => expect(loadSnapshotMock).toHaveBeenCalledTimes(1));
  // Give any stray effect run a tick to fire; the count must not move.
  await new Promise((resolve) => setTimeout(resolve, 10));
  expect(loadSnapshotMock).toHaveBeenCalledTimes(1);
  expect(loadSnapshotMock).toHaveBeenCalledWith("Crystals", undefined);
});

test("a stale snapshot response does not overwrite a newer one", async () => {
  const user = userEvent.setup();
  loadSnapshotMock.mockReset();
  let resolveStale: (value: AdminSnapshot) => void = () => {};
  const stalePending = new Promise<AdminSnapshot>((resolve) => {
    resolveStale = resolve;
  });
  loadSnapshotMock
    .mockReturnValueOnce(stalePending) // onMount load (Crystals) — hangs
    .mockResolvedValueOnce(snapshotFor("Concepts")); // second load — resolves first

  render(MemoryViews, {
    props: {
      dashboard: { ...dashboard, views: ["Crystals", "Concepts"] },
      onNotice: vi.fn(),
    },
  });
  await user.click(screen.getByRole("button", { name: "Concepts" }));
  await screen.findByText("Concepts record one");

  // The earlier, slower Crystals response now lands — it must be dropped.
  resolveStale(listSnapshot);
  await new Promise((resolve) => setTimeout(resolve, 10));
  expect(screen.queryByText("Crystal Alpha")).toBeNull();
  await screen.findByText("Concepts record one");
});

test("a non-destructive action posts the canonical id and selected row immediately", async () => {
  const user = userEvent.setup();
  runActionMock.mockResolvedValue({
    result: { message: "Crystal reinforced" },
    snapshot: listSnapshot.snapshot,
  } satisfies AdminActionResult);

  render(MemoryViews, { props: { dashboard, onNotice: vi.fn() } });
  await user.click(
    await screen.findByRole("button", { name: /Crystal Alpha/ }),
  );
  await screen.findByText("Evidence");
  await user.click(screen.getByRole("button", { name: "Reinforce Crystal" }));
  expect(runActionMock).toHaveBeenCalledWith("reinforce_crystal", {
    view: "Crystals",
    id: 7,
  });
});

test("a destructive action opens a dialog and only posts after explicit confirmation", async () => {
  const user = userEvent.setup();
  runActionMock.mockResolvedValue({
    result: { message: "Crystal deleted" },
    snapshot: listSnapshot.snapshot,
  } satisfies AdminActionResult);

  render(MemoryViews, { props: { dashboard, onNotice: vi.fn() } });
  await user.click(
    await screen.findByRole("button", { name: /Crystal Alpha/ }),
  );
  await screen.findByText("Evidence");
  await user.click(screen.getByRole("button", { name: "Delete Selected" }));
  // The dialog is open; nothing posted yet.
  expect(runActionMock).not.toHaveBeenCalled();
  await user.click(
    screen.getByLabelText(/apply this change to the stored memory/i),
  );
  await user.click(
    screen.getAllByRole("button", { name: "Delete Selected" }).at(-1)!,
  );
  expect(runActionMock).toHaveBeenCalledWith("delete_selected", {
    view: "Crystals",
    ids: [7],
    confirmed: true,
  });
});
