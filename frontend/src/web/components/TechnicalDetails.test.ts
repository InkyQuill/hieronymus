import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import TechnicalDetails from "./TechnicalDetails.svelte";
import SettingsSaveState from "./SettingsSaveState.svelte";

test("technical data can be opened and copied with the original field names", async () => {
  const user = userEvent.setup();
  const copy = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  const data = { pending_count: 37, state: "IDLE" };
  render(TechnicalDetails, { data });
  expect(
    (
      screen
        .getByText("Technical details", { exact: true })
        .closest("details") as HTMLDetailsElement
    ).open,
  ).toBe(false);
  await user.click(screen.getByText("Technical details", { exact: true }));
  await user.click(
    screen.getByRole("button", { name: "Copy technical details" }),
  );
  expect(copy).toHaveBeenCalledWith(JSON.stringify(data, null, 2));
  expect(screen.getByRole("status").textContent).toBe("Details copied.");
});

test("clipboard failures leave technical text available for manual copying", async () => {
  const user = userEvent.setup();
  vi.spyOn(navigator.clipboard, "writeText").mockRejectedValue(
    new Error("blocked"),
  );
  render(TechnicalDetails, { data: { pending_count: 37 } });
  await user.click(screen.getByText("Technical details", { exact: true }));
  await user.click(
    screen.getByRole("button", { name: "Copy technical details" }),
  );
  expect(screen.getByRole("alert").textContent).toContain("Select and copy");
  expect(screen.getByText(/"pending_count": 37/)).toBeTruthy();
});

test("save status follows edits and the saved server response", async () => {
  const { rerender } = render(SettingsSaveState, {
    current: { interval: 30 },
    saved: { interval: 30 },
  });
  expect(screen.getByRole("status").textContent).toBe(
    "Matches saved settings.",
  );
  await rerender({ current: { interval: 60 }, saved: { interval: 30 } });
  expect(screen.getByRole("status").textContent).toContain("Unsaved changes");
  await rerender({ current: { interval: 60 }, saved: { interval: 60 } });
  expect(screen.getByRole("status").textContent).toBe(
    "Matches saved settings.",
  );
});
