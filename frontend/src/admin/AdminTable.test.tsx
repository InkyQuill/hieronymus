import { afterEach, describe, expect, it } from "bun:test";
import React from "react";
import stringWidth from "string-width";
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

function displayColumnOf(line: string, value: string): number {
  const index = line.indexOf(value);
  expect(index).toBeGreaterThanOrEqual(0);
  return stringWidth(line.slice(0, index));
}

afterEach(async () => {
  await cleanupOpenTuiHarnesses();
});

describe("AdminTable", () => {
  it("shows scrollbar arrows when rows overflow the visible height", async () => {
    const rows = Array.from({ length: 30 }, (_, index) =>
      row({ id: index, label: `Row ${index}` }),
    );
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={40} height={5} />,
    );

    const output = await waitForFrame((frame) => frame.includes("Row 0"));
    expect(output).toContain("▲");
  });

  it("shows no scrollbar arrows when rows fit within the visible height", async () => {
    const rows = [row({ id: 1, label: "Only Row" })];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={40} height={5} />,
    );

    const output = await waitForFrame((frame) => frame.includes("Only Row"));
    expect(output).not.toContain("▲");
  });

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

  it("aligns CJK labels by terminal display cells", async () => {
    const rows = [
      row({ id: 1, label: "Short", status: "active", quality_label: "high" }),
      row({
        id: 2,
        label: "東京の記録",
        status: "pending",
        quality_label: "medium",
      }),
    ];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={40} height={6} />,
    );

    const output = await waitForFrame((frame) => frame.includes("東京の記録"));
    const lines = output.split("\n").map((line) => line.trimEnd());
    const shortLine = lines.find((line) => line.includes("Short"));
    const cjkLine = lines.find((line) => line.includes("東京の記録"));
    expect(shortLine).toBeDefined();
    expect(cjkLine).toBeDefined();
    expect(displayColumnOf(cjkLine!, "pending")).toBe(
      displayColumnOf(shortLine!, "active"),
    );
    expect(stringWidth(cjkLine!)).toBeLessThanOrEqual(40);
  });

  it("keeps emoji and combining graphemes intact while aligning display cells", async () => {
    const emoji = "👩🏽‍💻";
    const combining = "e\u0301e\u0301";
    const rows = [
      row({
        id: 1,
        label: emoji.repeat(3),
        status: "active",
        quality_label: "high",
      }),
      row({
        id: 2,
        label: combining,
        status: "pending",
        quality_label: "medium",
      }),
    ];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={28} height={6} />,
    );

    const output = await waitForFrame((frame) => frame.includes(combining));
    const lines = output.split("\n").map((line) => line.trimEnd());
    const emojiLine = lines.find((line) => line.includes(emoji));
    const combiningLine = lines.find((line) => line.includes(combining));
    expect(emojiLine).toBeDefined();
    expect(combiningLine).toBeDefined();
    expect(emojiLine).toContain(`${emoji}…`);
    expect(displayColumnOf(emojiLine!, "active")).toBe(
      displayColumnOf(combiningLine!, "pending"),
    );
    expect(stringWidth(emojiLine!)).toBeLessThanOrEqual(28);
    expect(stringWidth(combiningLine!)).toBeLessThanOrEqual(28);
  });

  it("shrinks metadata columns to stay within widths below 28 cells", async () => {
    const rows = [
      row({
        label: "Guild Ledger",
        status: "pending",
        quality_label: "medium",
      }),
    ];
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 40,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={24} height={4} />,
    );

    const output = await waitForFrame((frame) => frame.includes("pending"));
    const line = output
      .split("\n")
      .map((candidate) => candidate.trimEnd())
      .find((candidate) => candidate.includes("pending"));
    expect(line).toBeDefined();
    expect(line).toContain("medium");
    expect(stringWidth(line!)).toBeLessThanOrEqual(24);
  });

  it("collapses rows to the supplied width when fewer than four cells are available", async () => {
    const expectedRows = new Map([
      [1, ">"],
      [2, ">…"],
      [3, ">G…"],
    ]);

    for (const [width, expected] of expectedRows) {
      const { render, waitForFrame } = createOpenTuiHarness({
        width: 10,
        height: 4,
      });

      await render(
        <AdminTable
          rows={[row({ label: "Guild Ledger" })]}
          selectedId={1}
          width={width}
          height={2}
        />,
      );

      const output = await waitForFrame((frame) => frame.includes(">"));
      const line = output
        .split("\n")
        .find((candidate) => candidate.includes(">"));
      expect(line?.trimEnd()).toBe(expected);
      expect(stringWidth(line?.trimEnd() ?? "")).toBeLessThanOrEqual(width);
      await cleanupOpenTuiHarnesses();
    }
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
