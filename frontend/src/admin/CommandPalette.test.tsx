import { afterEach, describe, expect, it } from "bun:test";
import React from "react";
import type { AdminCommand } from "../rpc/schema.js";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { CommandPalette } from "./CommandPalette.js";

function command(
  overrides: Partial<AdminCommand & { disabled: boolean }> = {},
): AdminCommand & { disabled: boolean } {
  return {
    id: "edit_memory",
    label: "Edit Memory",
    hint: "Edit the selected crystal or lesson text.",
    key: "e",
    group: "Memory",
    views: ["Crystals"],
    requires_selection: true,
    disabled: false,
    ...overrides,
  };
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("CommandPalette", () => {
  it("marks a disabled command with a non-color marker instead of an inline suffix", async () => {
    const commands = [
      command({ id: "add_memory", label: "Add Memory", disabled: false }),
      command({ disabled: true }),
    ];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 12,
    });

    await render(<CommandPalette commands={commands} selectedIndex={0} />);

    const output = await waitForFrame((frame) => frame.includes("Edit Memory"));
    expect(output).toContain("✕ Edit Memory");
    expect(output).not.toContain("(unavailable)");
  });

  it("shows the disabled reason on the hint line only when the disabled command is highlighted", async () => {
    const commands = [command({ disabled: true })];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 12,
    });

    await render(<CommandPalette commands={commands} selectedIndex={0} />);

    const output = await waitForFrame((frame) => frame.includes("Edit Memory"));
    expect(output).toContain("Edit Memory needs a selected row");
  });

  it("shows the normal hint when the highlighted command is enabled", async () => {
    const commands = [command({ disabled: false })];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 12,
    });

    await render(<CommandPalette commands={commands} selectedIndex={0} />);

    const output = await waitForFrame((frame) => frame.includes("Edit Memory"));
    expect(output).toContain("Edit the selected crystal or lesson text.");
  });
});
