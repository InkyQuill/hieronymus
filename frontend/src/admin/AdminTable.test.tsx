import { afterEach, describe, expect, it } from "bun:test";
import { useKeyboard } from "@opentui/react";
import React, { useState } from "react";
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

function ResizableAdminTable({ rows }: { rows: AdminRow[] }) {
  const [height, setHeight] = useState(2);
  useKeyboard((key) => {
    if (key.name === "r") {
      setHeight(5);
    }
  });
  return (
    <AdminTable rows={rows} selectedId={null} width={40} height={height} />
  );
}

function SelectableAdminTable({ rows }: { rows: AdminRow[] }) {
  const [selectedId, setSelectedId] = useState<number>(0);
  useKeyboard((key) => {
    if (key.name === "s") {
      setSelectedId(20);
    }
  });
  return (
    <AdminTable rows={rows} selectedId={selectedId} width={40} height={5} />
  );
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
    expect(output).toContain("▼");
    const arrowLines = output
      .split("\n")
      .filter((line) => line.includes("▲") || line.includes("▼"));
    expect(arrowLines).toHaveLength(2);
    for (const line of arrowLines) {
      expect(displayColumnOf(line, line.includes("▲") ? "▲" : "▼")).toBe(39);
    }
  });

  it("preserves the full content area when rows exactly fit the visible height", async () => {
    const rows = Array.from({ length: 5 }, (_, index) =>
      row({ id: index, label: `Exact Row ${index}` }),
    );
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={40} height={5} />,
    );

    const output = await waitForFrame((frame) => frame.includes("Exact Row 4"));
    expect(output).not.toContain("▲");
    expect(output).not.toContain("▼");
    for (const index of rows.keys()) {
      expect(output).toContain(`Exact Row ${index}`);
    }
  });

  it("scrolls the selected row into view", async () => {
    const rows = Array.from({ length: 30 }, (_, index) =>
      row({ id: index, label: `Selected Row ${index}` }),
    );
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(
      <AdminTable rows={rows} selectedId={20} width={40} height={5} />,
    );

    const output = await waitForFrame((frame) =>
      frame.includes("Selected Row 20"),
    );
    expect(output).toContain("> Selected Row 20");
  });

  it("scrolls a newly selected row into view after mount", async () => {
    const rows = Array.from({ length: 30 }, (_, index) =>
      row({ id: index, label: `Changed Row ${index}` }),
    );
    const { render, waitForFrame, mockInput } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(<SelectableAdminTable rows={rows} />);
    await waitForFrame((frame) => frame.includes("> Changed Row 0"));
    await mockInput.press("s");

    const output = await waitForFrame((frame) =>
      frame.includes("> Changed Row 20"),
    );
    expect(output).not.toContain("> Changed Row 0");
  });

  it("restores automatic scrollbar visibility after growing from height two", async () => {
    const rows = Array.from({ length: 30 }, (_, index) =>
      row({ id: index, label: `Resized Row ${index}` }),
    );
    const { render, waitForFrame, mockInput } = createOpenTuiHarness({
      width: 60,
      height: 10,
    });

    await render(<ResizableAdminTable rows={rows} />);
    const small = await waitForFrame((frame) =>
      frame.includes("Resized Row 0"),
    );
    expect(small).not.toContain("▲");
    await mockInput.press("r");

    const grown = await waitForFrame((frame) =>
      frame.includes("Resized Row 4"),
    );
    expect(grown).toContain("▲");
    expect(grown).toContain("▼");
  });

  it("keeps rows visible at heights one and two without arrow controls", async () => {
    const rows = Array.from({ length: 3 }, (_, index) =>
      row({ id: index, label: `Tiny Row ${index}` }),
    );

    for (const height of [1, 2]) {
      const { render, waitForFrame } = createOpenTuiHarness({
        width: 60,
        height: 4,
      });
      await render(
        <AdminTable rows={rows} selectedId={null} width={40} height={height} />,
      );

      const output = await waitForFrame((frame) =>
        frame.includes("Tiny Row 0"),
      );
      if (height === 2) {
        expect(output).toContain("Tiny Row 1");
      }
      expect(output).not.toContain("▲");
      expect(output).not.toContain("▼");
      await cleanupOpenTuiHarnesses();
    }
  });

  it("shows right-edge arrows at the minimum three-row control height", async () => {
    const rows = Array.from({ length: 4 }, (_, index) =>
      row({ id: index, label: `Three High ${index}` }),
    );
    const { render, waitForFrame } = createOpenTuiHarness({
      width: 60,
      height: 5,
    });

    await render(
      <AdminTable rows={rows} selectedId={null} width={40} height={3} />,
    );

    const output = await waitForFrame((frame) =>
      frame.includes("Three High 2"),
    );
    for (const glyph of ["▲", "▼"]) {
      const line = output
        .split("\n")
        .find((candidate) => candidate.includes(glyph));
      expect(line).toBeDefined();
      expect(displayColumnOf(line!, glyph)).toBe(39);
    }
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
