import { afterEach, describe, expect, it } from "bun:test";
import React from "react";
import type { AdminRow } from "../rpc/schema.js";
import {
  cleanupOpenTuiHarnesses,
  createOpenTuiHarness,
} from "../test/opentuiHarness.js";
import { AdminTable } from "./AdminTable.js";

function row(overrides: Partial<AdminRow> = {}): AdminRow {
  return {
    id: 1,
    kind: "concept",
    label: "Guild Ledger",
    status: "active",
    scope: "only-sense-online",
    language_pair: "ja -> ru",
    quality_label: "high",
    tags: [],
    ...overrides,
  };
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("AdminTable", () => {
  it("aligns the status column at the same position across rows regardless of label length", async () => {
    const rows = [
      row({ id: 1, label: "Short", status: "active", quality_label: "high" }),
      row({
        id: 2,
        label: "A much longer label value",
        status: "pending",
        quality_label: "medium",
      }),
    ];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 80,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={60} height={6} />,
    );

    const output = await waitForFrame((frame) => frame.includes("Short"));
    const lines = output.split("\n").map((line) => line.trimEnd());
    const shortLine = lines.find((line) => line.includes("Short"));
    const longLine = lines.find((line) => line.includes("A much longer"));
    expect(shortLine).toBeDefined();
    expect(longLine).toBeDefined();
    const statusColumnStart = shortLine!.indexOf("active");
    expect(longLine!.indexOf("pending")).toBe(statusColumnStart);
  });

  it("truncates a label that exceeds the column width with an ellipsis", async () => {
    const rows = [
      row({
        id: 1,
        label: "A".repeat(80),
        status: "active",
        quality_label: "high",
      }),
    ];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={40} height={4} />,
    );

    const output = await waitForFrame((frame) => frame.includes("…"));
    expect(output).toContain("…");
    expect(output).not.toContain("A".repeat(80));
  });

  it("keeps the selection marker for the highlighted row", async () => {
    const rows = [row({ id: 1, label: "Guild Ledger" })];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={1} focused width={40} height={4} />,
    );

    const output = await waitForFrame((frame) =>
      frame.includes("Guild Ledger"),
    );
    expect(output).toContain("> Guild Ledger");
  });
});
