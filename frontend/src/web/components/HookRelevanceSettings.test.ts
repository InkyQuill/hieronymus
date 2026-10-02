import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import HookRelevanceSettings from "./HookRelevanceSettings.svelte";
import { loadRelevanceSettings, saveRelevanceSettings } from "../lib/api";
vi.mock("../lib/api", () => ({
  loadRelevanceSettings: vi.fn(),
  saveRelevanceSettings: vi.fn(),
}));
const settings = {
  key_configured: true,
  model: "jev-1.13.0",
  minimum_relevance: 0.85,
  maximum_technical: 0.15,
  timeout_seconds: 5,
};
test("saved credentials stay masked, blank preserves and explicit removal clears", async () => {
  vi.mocked(loadRelevanceSettings).mockResolvedValue({ ...settings });
  vi.mocked(saveRelevanceSettings).mockResolvedValue({ ...settings });
  const user = userEvent.setup();
  render(HookRelevanceSettings);
  const key = await screen.findByLabelText("TypeSafe API key");
  expect(key.getAttribute("type")).toBe("password");
  expect((key as HTMLInputElement).value).toBe("");
  await user.click(screen.getByRole("button", { name: "Save relevance" }));
  expect(saveRelevanceSettings).toHaveBeenLastCalledWith({
    model: settings.model,
    minimum_relevance: 0.85,
    maximum_technical: 0.15,
    timeout_seconds: 5,
    api_key: "",
    clear_key: false,
  });
  await screen.findByRole("status");
  await user.click(screen.getByLabelText("Remove saved Jev key"));
  vi.mocked(saveRelevanceSettings).mockResolvedValue({
    ...settings,
    key_configured: false,
  });
  await user.click(screen.getByRole("button", { name: "Save relevance" }));
  expect(saveRelevanceSettings).toHaveBeenLastCalledWith(
    expect.objectContaining({ clear_key: true, api_key: "" }),
  );
  await screen.findByText("Local filter is active. No Jev API key is saved.");
});
test("service settings failures are visible", async () => {
  vi.mocked(loadRelevanceSettings).mockRejectedValue(
    new Error("invalid relevance.conf"),
  );
  render(HookRelevanceSettings);
  expect((await screen.findByRole("alert")).textContent).toBe(
    "invalid relevance.conf",
  );
});

test("new key clears on success and failed saves preserve editable draft", async () => {
  vi.mocked(loadRelevanceSettings).mockResolvedValue({ ...settings });
  vi.mocked(saveRelevanceSettings).mockRejectedValueOnce(
    new Error("Could not save"),
  );
  const user = userEvent.setup();
  render(HookRelevanceSettings);
  const key = (await screen.findByLabelText(
    "TypeSafe API key",
  )) as HTMLInputElement;
  await user.type(key, "synthetic-key");
  await user.click(screen.getByRole("button", { name: "Save relevance" }));
  await screen.findByRole("alert");
  expect(key.value).toBe("synthetic-key");
  expect(key.disabled).toBe(false);
  vi.mocked(saveRelevanceSettings).mockResolvedValue({ ...settings });
  await user.click(screen.getByRole("button", { name: "Save relevance" }));
  await screen.findByRole("status");
  expect(key.value).toBe("");
  expect(saveRelevanceSettings).toHaveBeenLastCalledWith(
    expect.objectContaining({ api_key: "synthetic-key" }),
  );
  await user.type(key, "another-key");
  expect(screen.queryByRole("status")).toBeNull();
});

test("failed initial relevance load can be retried locally", async () => {
  vi.mocked(loadRelevanceSettings).mockRejectedValueOnce(new Error("Offline"));
  const user = userEvent.setup();
  render(HookRelevanceSettings);
  await screen.findByRole("alert");
  vi.mocked(loadRelevanceSettings).mockResolvedValue({ ...settings });
  await user.click(
    screen.getByRole("button", { name: "Retry relevance settings" }),
  );
  expect(await screen.findByLabelText("TypeSafe API key")).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});
