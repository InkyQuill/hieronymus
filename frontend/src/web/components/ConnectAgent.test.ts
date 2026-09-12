import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import ConnectAgent from "./ConnectAgent.svelte";
import { prepareAgentConnection } from "../lib/api";

vi.mock("../lib/api", () => ({ prepareAgentConnection: vi.fn() }));

test("preparation enables the selected host request without claiming installation", async () => {
  const user = userEvent.setup();
  vi.mocked(prepareAgentConnection).mockResolvedValue([
    { id: "codex", name: "Codex", instructions: "Add Codex MCP and skills" },
    { id: "pi", name: "pi", instructions: "Add pi MCP and skills" },
  ]);
  render(ConnectAgent);
  const copy = screen.getByRole("button", { name: "Copy setup request" });
  expect((copy as HTMLButtonElement).disabled).toBe(true);
  await user.click(screen.getByRole("button", { name: "Prepare connection" }));
  await screen.findByLabelText("Setup request");
  await user.click(screen.getByRole("radio", { name: "pi" }));
  await user.click(copy);
  expect(await navigator.clipboard.readText()).toBe("Add pi MCP and skills");
  expect(screen.getByRole("status").textContent).toContain(
    "Paste the request into pi",
  );
  expect(
    screen.getByText(/Preparing or copying a request does not verify/),
  ).toBeTruthy();
});
