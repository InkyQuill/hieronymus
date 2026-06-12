import React from "react";
import type { AdminRow } from "../rpc/schema.js";

export function AdminTable({
  rows,
  selectedId,
  focused = true,
}: {
  rows: AdminRow[];
  selectedId: string | number | null;
  focused?: boolean;
}) {
  return (
    <scrollbox flexDirection="column" width={48} height={18}>
      {rows.map((row) => (
        <text
          key={String(row.id)}
          fg={
            row.id === selectedId ? (focused ? "cyan" : "white") : undefined
          }
        >
          {row.id === selectedId ? ">" : " "} {row.label} [{row.status}]{" "}
          {row.quality_label}
        </text>
      ))}
    </scrollbox>
  );
}
