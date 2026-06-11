import React from "react";
import { Text } from "ink";

type Props = {
  message: string;
  error?: boolean;
};

export function StatusLine({ message, error = false }: Props) {
  return <Text color={error ? "red" : "green"}>{message}</Text>;
}
