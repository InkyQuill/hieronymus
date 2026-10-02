import { render, screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import AdminDashboard from "./AdminDashboard.svelte";
import SettingsNavigation from "./SettingsNavigation.svelte";

test("overview reports disabled scheduling and the actual pending count", () => {
  render(AdminDashboard, {
    dashboard: {
      header: {
        product: "Hieronymus",
        version: "0.10.1",
        tagline: "Local memory",
      },
      stats: {},
      views: [],
      short_term_status: { pending_count: 37 },
      dream_status: { state: "DISABLED" },
    },
    onDream: vi.fn(),
    busy: true,
  });
  expect(screen.getByText("37 awaiting processing")).toBeTruthy();
  expect(screen.queryByText("Ready for the next run")).toBeNull();
  expect(
    screen
      .getByRole("button", { name: "Process memories now" })
      .hasAttribute("disabled"),
  ).toBe(true);
});

test("settings navigation identifies the current page consistently", () => {
  render(SettingsNavigation, { section: "ingest" });
  expect(
    screen
      .getByRole("link", { name: "Incoming memory" })
      .getAttribute("aria-current"),
  ).toBe("page");
  expect(
    screen
      .getByRole("link", { name: "AI providers" })
      .hasAttribute("aria-current"),
  ).toBe(false);
});
