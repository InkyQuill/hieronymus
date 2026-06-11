import React from "react";
import { Box, Text } from "ink";

type Props<T> = {
  items: T[];
  selectedIndex: number;
  label: (item: T) => string;
  focused?: boolean;
};

export function FocusableList<T>({
  items,
  selectedIndex,
  label,
  focused = true,
}: Props<T>) {
  return (
    <Box flexDirection="column">
      {items.map((item, index) => (
        <Text
          key={`${index}-${label(item)}`}
          color={
            index === selectedIndex ? (focused ? "cyan" : "white") : undefined
          }
        >
          {index === selectedIndex ? ">" : " "} {label(item)}
        </Text>
      ))}
    </Box>
  );
}
