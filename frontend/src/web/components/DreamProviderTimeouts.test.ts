import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import DreamProviderTimeouts from "./DreamProviderTimeouts.svelte";
import { listProviders, saveProvider } from "../lib/api";
import type { ProviderProfile } from "../lib/types";

vi.mock("../lib/api", () => ({
  listProviders: vi.fn(),
  saveProvider: vi.fn(),
}));
const provider: ProviderProfile = {
  id: "deepseek",
  name: "DeepSeek",
  type: "openai",
  url: "https://api.deepseek.com",
  key_configured: true,
  model: "deepseek-flash",
  timeout_seconds: 60,
};

test("Dreaming saves a long timeout while preserving the current provider profile", async () => {
  const current = {
    ...provider,
    name: "Renamed DeepSeek",
    context_window: 128000,
  };
  vi.mocked(listProviders).mockResolvedValue([current]);
  vi.mocked(saveProvider).mockResolvedValue({
    ...current,
    timeout_seconds: 600,
  });
  const user = userEvent.setup();
  render(DreamProviderTimeouts, {
    providers: [provider],
    providerIds: [provider.id],
  });
  const input = screen.getByLabelText("DeepSeek timeout (seconds)");
  expect(input.hasAttribute("max")).toBe(false);
  await user.clear(input);
  await user.type(input, "600");
  await user.click(
    screen.getByRole("button", { name: "Save DeepSeek timeout" }),
  );
  await screen.findByRole("status");
  expect(saveProvider).toHaveBeenCalledWith({
    id: "deepseek",
    name: current.name,
    type: current.type,
    url: current.url,
    key: "",
    timeout_seconds: "600",
    context_window: "128000",
  });
});

test("failed timeout save leaves the edited value available for retry", async () => {
  vi.mocked(listProviders).mockResolvedValue([provider]);
  vi.mocked(saveProvider).mockRejectedValue(new Error("Cannot save profile"));
  const user = userEvent.setup();
  render(DreamProviderTimeouts, {
    providers: [provider],
    providerIds: [provider.id],
  });
  const input = screen.getByLabelText(
    "DeepSeek timeout (seconds)",
  ) as HTMLInputElement;
  await user.clear(input);
  await user.type(input, "300");
  await user.click(
    screen.getByRole("button", { name: "Save DeepSeek timeout" }),
  );
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Cannot save",
  );
  expect(input.value).toBe("300");
  expect(input.disabled).toBe(false);
});

test("provider refresh updates untouched timeouts without erasing an edited draft", async () => {
  const user = userEvent.setup();
  const { rerender } = render(DreamProviderTimeouts, {
    providers: [provider],
    providerIds: [provider.id],
  });
  const input = screen.getByLabelText(
    "DeepSeek timeout (seconds)",
  ) as HTMLInputElement;
  await rerender({
    providers: [{ ...provider, timeout_seconds: 300 }],
    providerIds: [provider.id],
  });
  expect(input.value).toBe("300");
  await user.clear(input);
  await user.type(input, "600");
  await rerender({
    providers: [{ ...provider, timeout_seconds: 400 }],
    providerIds: [provider.id],
  });
  expect(input.value).toBe("600");
});
