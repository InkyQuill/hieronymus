import React from "react";
import { Box, Text } from "ink";

const COMMANDS: Record<string, string[]> = {
  Crystals: [
    "add",
    "edit",
    "delete",
    "merge",
    "split",
    "deprecate",
    "supersede",
    "reinforce",
    "decay",
    "inspect provenance",
    "inspect recall reason",
  ],
  Lessons: [
    "add",
    "edit",
    "delete",
    "merge",
    "split",
    "deprecate",
    "supersede",
    "reinforce",
    "decay",
    "promote local lesson",
    "activate global lesson",
    "inspect provenance",
    "inspect recall reason",
  ],
  "Dream Runs": ["run manual dreaming", "review dream outputs"],
  Proposals: ["approve", "reject"],
};

export function commandsForView(view: string): string[] {
  return COMMANDS[view] ?? [];
}

export function CommandPalette({ view }: { view: string }) {
  return (
    <Box flexDirection="column">
      <Text bold>Commands</Text>
      {commandsForView(view).map((command) => (
        <Text key={command}>{command}</Text>
      ))}
    </Box>
  );
}
