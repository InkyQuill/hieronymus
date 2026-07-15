import { afterEach, describe, expect, it } from "bun:test";
import React from "react";
import { useKeyboard } from "@opentui/react";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { useFieldFocus } from "./useFieldFocus.js";

function Probe({ fieldCount }: { fieldCount: number }) {
  const { focusedIndex, moveUp, moveDown } = useFieldFocus(fieldCount);
  useKeyboard((key) => {
    if (key.name === "down") {
      moveDown();
    } else if (key.name === "up") {
      moveUp();
    }
  });
  return <text>focused {focusedIndex}</text>;
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("useFieldFocus", () => {
  it("starts at index 0", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Probe fieldCount={3} />);

    const output = await waitForFrame((frame) => frame.includes("focused"));
    expect(output).toContain("focused 0");
  });

  it("moves down and clamps at fieldCount - 1", async () => {
    const { render, mockInput, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Probe fieldCount={2} />);
    await mockInput.press("down");
    await mockInput.press("down");
    await mockInput.press("down");

    const output = await waitForFrame((frame) => frame.includes("focused 1"));
    expect(output).toContain("focused 1");
  });

  it("moves up and clamps at 0", async () => {
    const { render, mockInput, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Probe fieldCount={3} />);
    await mockInput.press("down");
    await mockInput.press("up");
    await mockInput.press("up");

    const output = await waitForFrame((frame) => frame.includes("focused 0"));
    expect(output).toContain("focused 0");
  });
});
