import React from "react";
import type { AdminRow } from "../rpc/schema.js";

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
  return (
    <scrollbox flexDirection="column" width={width} height={height}>
      {rows.map((row) => (
        <text
          key={String(row.id)}
          fg={row.id === selectedId ? (focused ? "cyan" : "white") : undefined}
        >
          {row.id === selectedId ? ">" : " "} {row.label} [{row.status}]{" "}
          {row.quality_label}
        </text>
      ))}
    </scrollbox>
  );
}
