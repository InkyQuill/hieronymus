import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import MemoryComparisonSettings from "./MemoryComparisonSettings.svelte";
import { loadComparisonSettings, saveComparisonSettings } from "../lib/api";
import type { ComparisonState } from "../lib/types";
vi.mock("../lib/api", () => ({
  loadComparisonSettings: vi.fn(),
  saveComparisonSettings: vi.fn(),
}));
const state: ComparisonState = {
  settings: {
    primary: { provider: "jev", model: "jev-1.13.0" },
    fallback: null,
    max_pairs_per_run: 8,
    timeout_seconds: 10,
  },
  primary_ready: true,
  fallback_ready: false,
  qualified: false,
};
test("fallback is explicit and disabling comparison clears both assignments", async () => {
  vi.mocked(loadComparisonSettings).mockResolvedValue({
    comparison: structuredClone(state),
    providers: [],
  });
  vi.mocked(saveComparisonSettings).mockResolvedValue(structuredClone(state));
  const user = userEvent.setup();
  render(MemoryComparisonSettings);
  const backup = await screen.findByLabelText("Backup comparison provider");
  expect((backup as HTMLSelectElement).value).toBe("");
  await user.selectOptions(backup, "jev");
  await user.type(
    screen.getByLabelText("Backup comparison model"),
    "jev-backup",
  );
  await user.click(
    screen.getByRole("button", { name: "Save memory comparison" }),
  );
  expect(saveComparisonSettings).toHaveBeenLastCalledWith(
    expect.objectContaining({
      fallback: { provider: "jev", model: "jev-backup" },
    }),
  );
  await screen.findByRole("status");
  await user.selectOptions(
    screen.getByLabelText("Primary comparison provider"),
    "",
  );
  await user.click(
    screen.getByRole("button", { name: "Save memory comparison" }),
  );
  expect(saveComparisonSettings).toHaveBeenLastCalledWith(
    expect.objectContaining({ primary: null, fallback: null }),
  );
});
test("failed save preserves the draft and reports the error", async () => {
  vi.mocked(loadComparisonSettings).mockResolvedValue({
    comparison: structuredClone(state),
    providers: [],
  });
  vi.mocked(saveComparisonSettings).mockRejectedValue(
    new Error("Cannot save comparison.conf"),
  );
  const user = userEvent.setup();
  render(MemoryComparisonSettings);
  const model = await screen.findByLabelText("Primary comparison model");
  await user.clear(model);
  await user.type(model, "new-model");
  await user.click(
    screen.getByRole("button", { name: "Save memory comparison" }),
  );
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Cannot save",
  );
  expect((model as HTMLInputElement).value).toBe("new-model");
  expect((model as HTMLInputElement).disabled).toBe(false);
});

test("changing either provider clears its previous model", async () => {
  const configured = structuredClone(state);
  configured.settings.fallback = { provider: "jev", model: "old-backup" };
  vi.mocked(loadComparisonSettings).mockResolvedValue({
    comparison: configured,
    providers: [],
  });
  const user = userEvent.setup();
  render(MemoryComparisonSettings);
  for (const role of ["Primary", "Backup"]) {
    const provider = await screen.findByLabelText(
      `${role} comparison provider`,
    );
    await user.selectOptions(provider, "");
    await user.selectOptions(provider, "jev");
    expect(
      (screen.getByLabelText(`${role} comparison model`) as HTMLInputElement)
        .value,
    ).toBe("");
  }
});
