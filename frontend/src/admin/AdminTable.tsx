import React from "react";
import type { AdminRow } from "../rpc/schema.js";

const MARKER_WIDTH = 2;
const COLUMN_GAP = 1;
const STATUS_COLUMN_WIDTH = 10;
const QUALITY_COLUMN_WIDTH = 10;
const MIN_LABEL_COLUMN_WIDTH = 4;
const ELLIPSIS = "…";

export function AdminTable({
  rows,
  selectedId,
  focused = true,
  width = 48,
  height = 18,
}: {
  rows: AdminRow[];
  selectedId: string | number | null;
  focused?: boolean;
  width?: number;
  height?: number;
}) {
  const labelWidth = Math.max(
    MIN_LABEL_COLUMN_WIDTH,
    width -
      MARKER_WIDTH -
      COLUMN_GAP * 2 -
      STATUS_COLUMN_WIDTH -
      QUALITY_COLUMN_WIDTH,
  );

  return (
    <scrollbox flexDirection="column" width={width} height={height}>
      {rows.map((row) => (
        <text
          key={String(row.id)}
          fg={row.id === selectedId ? (focused ? "cyan" : "white") : undefined}
        >
          {row.id === selectedId ? "> " : "  "}
          {padColumn(row.label, labelWidth)}{" "}
          {padColumn(row.status, STATUS_COLUMN_WIDTH)}{" "}
          {padColumn(row.quality_label, QUALITY_COLUMN_WIDTH)}
        </text>
      ))}
    </scrollbox>
  );
}

function padColumn(value: string, columnWidth: number): string {
  if (value.length > columnWidth) {
    return columnWidth <= 1
      ? value.slice(0, columnWidth)
      : `${value.slice(0, columnWidth - 1)}${ELLIPSIS}`;
  }
  return value.padEnd(columnWidth, " ");
}
