import { afterEach, describe, expect, it } from "bun:test";
import { useKeyboard } from "@opentui/react";
import React, { useState } from "react";
import type { AdminDetail } from "../rpc/schema.js";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { DetailPane } from "./DetailPane.js";

function detail(overrides: Partial<AdminDetail> = {}): AdminDetail {
  return {
    title: "Guild Ledger",
    subtitle: "concept",
    body: "Guild ledger detail marker.",
    fields: [],
    ...overrides,
  };
}

function ResizableDetailPane({ value }: { value: AdminDetail }) {
  const [height, setHeight] = useState(2);
  useKeyboard((key) => {
    if (key.name === "r") {
      setHeight(5);
    }
  });
  return <DetailPane detail={value} width={40} height={height} />;
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("DetailPane", () => {
  it("shows scrollbar arrows when the body overflows the visible height", async () => {
    const longBody = Array.from({ length: 30 }, (_, i) => `Line ${i}`).join(
      "\n",
    );
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <DetailPane detail={detail({ body: longBody })} width={40} height={5} />,
    );

    const output = await waitForFrame((frame) => frame.includes("Line 0"));
    expect(output).toContain("▲");
    expect(output).toContain("▼");
    for (const glyph of ["▲", "▼"]) {
      const line = output
        .split("\n")
        .find((candidate) => candidate.includes(glyph));
      expect(line).toBeDefined();
      expect(line!.indexOf(glyph)).toBe(39);
    }
  });

  it("preserves the full content area when the body exactly fits", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <DetailPane
        detail={detail({ body: "Short body." })}
        width={40}
        height={5}
      />,
    );

    const output = await waitForFrame((frame) => frame.includes("Short body."));
    expect(output).not.toContain("▲");
    expect(output).not.toContain("▼");
    expect(output).toContain("Guild Ledger");
    expect(output).toContain("concept");
  });

  it("restores automatic scrollbar visibility after growing from height two", async () => {
    const longBody = Array.from(
      { length: 30 },
      (_, i) => `Resize Line ${i}`,
    ).join("\n");
    const { render, waitForFrame, mockInput } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(<ResizableDetailPane value={detail({ body: longBody })} />);
    const small = await waitForFrame((frame) => frame.includes("Guild Ledger"));
    expect(small).not.toContain("▲");
    await mockInput.press("r");

    const grown = await waitForFrame((frame) =>
      frame.includes("Resize Line 0"),
    );
    expect(grown).toContain("▲");
    expect(grown).toContain("▼");
  });

  it("keeps header content visible at heights one and two without arrows", async () => {
    for (const height of [1, 2]) {
      const { render, waitForFrame } = createOpenTuiHarness({
        width: 60,
        height: 4,
      });
      await render(<DetailPane detail={detail()} width={40} height={height} />);

      const output = await waitForFrame(() => true);
      expect(output).toContain("Guild Ledger");
      if (height === 2) {
        expect(output).toContain("concept");
      }
      expect(output).not.toContain("▲");
      expect(output).not.toContain("▼");
      await cleanupOpenTuiHarnesses();
    }
  });

  it("shows right-edge arrows at the minimum three-row control height", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 5,
    });

    await render(<DetailPane detail={detail()} width={40} height={3} />);

    const output = await waitForFrame((frame) => frame.includes("concept"));
    for (const glyph of ["▲", "▼"]) {
      const line = output
        .split("\n")
        .find((candidate) => candidate.includes(glyph));
      expect(line).toBeDefined();
      expect(line!.indexOf(glyph)).toBe(39);
    }
  });
});
