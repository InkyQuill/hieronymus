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
  });

  it("shows no scrollbar arrows when the body fits within the visible height", async () => {
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <DetailPane
        detail={detail({ body: "Short body." })}
        width={40}
        height={10}
      />,
    );

    const output = await waitForFrame((frame) => frame.includes("Short body."));
    expect(output).not.toContain("▲");
  });
});
