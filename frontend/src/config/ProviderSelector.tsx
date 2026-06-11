import React from "react";
import { Box, Text } from "ink";
import type { ConfigBootstrap, ProviderName } from "../rpc/schema.js";

type Props = {
  choices: ConfigBootstrap["provider_choices"];
  selected: ProviderName;
  focused?: boolean;
};

export function ProviderSelector({ choices, selected, focused = true }: Props) {
  return (
    <Box flexDirection="column" width={24}>
      {choices.map((choice) => (
        <Text
          key={choice.name}
          color={
            choice.name === selected ? (focused ? "cyan" : "white") : undefined
          }
        >
          {choice.name === selected ? ">" : " "} {choice.display_name}
        </Text>
      ))}
    </Box>
  );
}
