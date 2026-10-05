import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import App from "./App.svelte";
import { loadAdminDashboard, loadVersion, startAdminDreaming } from "./lib/api";
import type { AdminDashboard } from "./lib/types";

vi.mock("./lib/theme.svelte", () => ({
  createThemeToggle: () => ({ theme: "dark", toggle: () => {} }),
}));
vi.mock("./lib/api", async (original) => ({
  ...(await original<object>()),
  loadAdminDashboard: vi.fn(),
  loadVersion: vi.fn(),
  startAdminDreaming: vi.fn(),
}));
vi.mock("./lib/admin-events.svelte", () => ({
  connectAdminEvents: () => () => {},
}));
const dashboard = (indexed: number): AdminDashboard => ({
  header: { product: "Hieronymus", version: "test", tagline: "" },
  stats: {},
  views: [],
  short_term_status: {},
  dream_status: { state: "idle" },
  memory_indexing: {
    state: "indexing",
    total: 100,
    indexed,
    pending: 100 - indexed,
    detail: null,
  },
});
afterEach(() => {
  vi.useRealTimers();
  window.history.replaceState({}, "", "/");
});

test.each(["success", "failure"])(
  "a stale poll %s cannot replace action state or hide its error",
  async (outcome) => {
    window.history.replaceState({}, "", "/admin");
    vi.useFakeTimers();
    vi.mocked(loadVersion).mockResolvedValue({ server_version: "test" });
    vi.mocked(loadAdminDashboard).mockResolvedValueOnce(dashboard(10));
    let resolvePoll!: (value: AdminDashboard) => void;
    let rejectPoll!: (reason: Error) => void;
    vi.mocked(loadAdminDashboard).mockImplementationOnce(
      () =>
        new Promise((resolve, reject) => {
          resolvePoll = resolve;
          rejectPoll = reject;
        }),
    );
    vi.mocked(startAdminDreaming).mockRejectedValue(
      new Error("Dreaming could not start"),
    );
    render(App);
    await vi.advanceTimersByTimeAsync(0);
    await tick();
    await vi.advanceTimersByTimeAsync(2_000);
    await fireEvent.click(
      screen.getByRole("button", { name: "Process memories now" }),
    );
    await tick();
    if (outcome === "success") resolvePoll(dashboard(1));
    else rejectPoll(new Error("Old poll failed"));
    await vi.advanceTimersByTimeAsync(0);
    await tick();
    expect(screen.getByText("Dreaming could not start")).toBeTruthy();
    expect(screen.getByText(/10 \/ 100 memories indexed/)).toBeTruthy();
    expect(screen.queryByText("Old poll failed")).toBeNull();
    vi.mocked(loadAdminDashboard).mockResolvedValue(dashboard(20));
    await vi.advanceTimersByTimeAsync(2_000);
    await tick();
    expect(screen.getByText(/20 \/ 100 memories indexed/)).toBeTruthy();
    expect(screen.getByText("Dreaming could not start")).toBeTruthy();
  },
);
