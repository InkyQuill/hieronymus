import React from "react";
import { Text } from "ink";

export function KeyHelp({ keys }: { keys: string[] }) {
  return <Text dimColor>{keys.join("  ")}</Text>;
}
