import { afterEach, describe, expect, it } from "bun:test";
import React from "react";
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

  it("keeps header content visible at heights too small for arrow controls", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 4,
    });

    await render(<DetailPane detail={detail()} width={40} height={2} />);

    const output = await waitForFrame(() => true);
    expect(output).toContain("Guild Ledger");
    expect(output).toContain("concept");
    expect(output).not.toContain("▲");
    expect(output).not.toContain("▼");
  });
});
