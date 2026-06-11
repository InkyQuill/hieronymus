import React from "react";
import { Box, Text } from "ink";
import type { AdminSnapshot } from "../rpc/schema.js";

export function DetailPane({ detail }: { detail: AdminSnapshot["detail"] }) {
  return (
    <Box flexDirection="column" width={60}>
      <Text bold>{detail.title}</Text>
      <Text dimColor>{detail.subtitle}</Text>
      <Text>{detail.body}</Text>
      {detail.fields.map(([name, value], index) => (
        <Text key={`${name}-${index}`}>
          {name}: {value}
        </Text>
      ))}
    </Box>
  );
}
