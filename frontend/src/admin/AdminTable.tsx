import React from "react";
import { Box, Text } from "ink";
import type { AdminRow } from "../rpc/schema.js";

export function AdminTable({
  rows,
  selectedId,
}: {
  rows: AdminRow[];
  selectedId: string | number | null;
}) {
  return (
    <Box flexDirection="column" width={48}>
      {rows.map((row) => (
        <Text
          key={String(row.id)}
          color={row.id === selectedId ? "cyan" : undefined}
        >
          {row.id === selectedId ? ">" : " "} {row.label} [{row.status}]{" "}
          {row.quality_label}
        </Text>
      ))}
    </Box>
  );
}
