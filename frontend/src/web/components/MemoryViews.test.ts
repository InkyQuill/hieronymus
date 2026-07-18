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

const dashboard = {
  header: {
    product: "Hieronymus",
    version: "0.4.0",
    tagline: "Translation memory",
  },
  stats: {},
  views: ["Crystals"],
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

test("destructive memory actions require confirmation and send the exact payload", async () => {
  const user = userEvent.setup();
  const actionResult = {
    result: { message: "Deleted Crystal Alpha." },
    snapshot: listSnapshot.snapshot,
  } satisfies AdminActionResult;
  runActionMock.mockResolvedValue(actionResult);

  render(MemoryViews, { props: { dashboard, onNotice: vi.fn() } });
  await user.click(
    await screen.findByRole("button", { name: /Crystal Alpha/ }),
  );
  await screen.findByText("Evidence");
  await user.click(screen.getByRole("button", { name: "Delete" }));
  expect(runActionMock).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm Delete" }));
  expect(runActionMock).toHaveBeenCalledWith("delete_crystal", {
    id: 7,
    confirmed: true,
  });
});

test("short-term memories require confirmation before removal and refresh after success", async () => {
  const user = userEvent.setup();
  const shortTermRow = {
    ...row,
    id: 11,
    kind: "short-term memory",
    label: "Transient phrase",
  };
  const shortTermSnapshot = {
    snapshot: {
      ...selectedSnapshot.snapshot,
      view: "Short-Term Memory",
      rows: [shortTermRow],
      selected: shortTermRow,
      detail: {
        ...selectedSnapshot.snapshot.detail,
        title: "Transient phrase",
      },
    },
  } satisfies AdminSnapshot;
  const refreshedSnapshot = {
    snapshot: {
      ...shortTermSnapshot.snapshot,
      rows: [],
      selected: null,
    },
  } satisfies AdminSnapshot;
  const shortTermDashboard = {
    ...dashboard,
    views: ["Short-Term Memory"],
  } satisfies AdminDashboard;
  let resolveRemoval!: (result: AdminActionResult) => void;

  loadSnapshotMock
    .mockReset()
    .mockResolvedValueOnce(shortTermSnapshot)
    .mockResolvedValueOnce(refreshedSnapshot);
  runActionMock.mockImplementation(
    () =>
      new Promise((resolve) => {
        resolveRemoval = resolve;
      }),
  );

  render(MemoryViews, {
    props: { dashboard: shortTermDashboard, onNotice: vi.fn() },
  });
  await user.click(await screen.findByRole("button", { name: "Remove" }));

  expect(runActionMock).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm Remove" }));
  expect(runActionMock).toHaveBeenCalledWith("remove_short_term_memory", {
    id: 11,
    confirmed: true,
  });
  expect(loadSnapshotMock).toHaveBeenCalledTimes(1);

  resolveRemoval({
    result: { message: "Removed Transient phrase." },
    snapshot: refreshedSnapshot.snapshot,
  });
  await waitFor(() =>
    expect(loadSnapshotMock).toHaveBeenLastCalledWith(
      "Short-Term Memory",
      undefined,
    ),
  );
});
