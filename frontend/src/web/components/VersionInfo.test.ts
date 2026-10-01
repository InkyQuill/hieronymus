import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import VersionInfo from "./VersionInfo.svelte";
import { loadVersion } from "../lib/api";
vi.mock("../lib/api", () => ({ loadVersion: vi.fn() }));
test("running server version remains copyable without checking releases", async () => {
  const user = userEvent.setup();
  vi.mocked(loadVersion).mockResolvedValue({ server_version: "9.8.7-dev.2" });
  render(VersionInfo);
  expect(await screen.findByText("Server v9.8.7-dev.2")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Copy version" }));
  expect(await navigator.clipboard.readText()).toBe(
    "Hieronymus server v9.8.7-dev.2",
  );
});
test("an unavailable server does not display the frontend build as its version", async () => {
  vi.mocked(loadVersion).mockRejectedValue(new Error("offline"));
  render(VersionInfo);
  expect(await screen.findByText("Server version unavailable")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Copy version" })).toBeNull();
});
