import { afterEach, describe, expect, it } from "bun:test";
import React from "react";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { Gauge } from "./Gauge.js";

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("Gauge", () => {
  it("renders a half-filled bar at 50%", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Gauge label="Short-term" value={5} max={10} barWidth={10} />);

    const output = await waitForFrame((frame) => frame.includes("Short-term"));
    expect(output).toContain("Short-term [█████░░░░░] 5/10");
  });

  it("rounds a partial fill to the nearest block", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Gauge label="Drain" value={1} max={3} barWidth={9} />);

    const output = await waitForFrame((frame) => frame.includes("Drain"));
    expect(output).toContain("Drain [███░░░░░░] 1/3");
  });

  it("renders a full bar at 100%", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Gauge label="Dream" value={4} max={4} barWidth={4} />);

    const output = await waitForFrame((frame) => frame.includes("Dream"));
    expect(output).toContain("Dream [████] 4/4");
  });

  it("falls back to a plain fraction when max is zero", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(<Gauge label="Queue" value={0} max={0} barWidth={10} />);

    const output = await waitForFrame((frame) => frame.includes("Queue"));
    expect(output).toContain("Queue 0/0");
    expect(output).not.toContain("[");
  });

  it("falls back to a plain fraction when the width is too small for a bar", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 5,
    });

    await render(
      <Gauge label="Q" value={2} max={4} barWidth={10} width={10} />,
    );

    const output = await waitForFrame((frame) => frame.includes("Q "));
    expect(output).toContain("Q 2/4");
    expect(output).not.toContain("[");
  });
});
