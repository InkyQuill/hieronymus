import React from "react";
import { Spinner } from "./Spinner.js";
import { theme } from "./theme.js";

type Props = {
  message: string;
  error?: boolean;
  busy?: boolean;
};

export function StatusLine({ message, error = false, busy = false }: Props) {
  const fg = error
    ? theme.statusError
    : busy
      ? theme.accentPrimary
      : theme.statusSuccess;
  return (
    <box flexDirection="row" marginTop={1}>
      {busy && (
        <box marginRight={1}>
          <Spinner />
        </box>
      )}
      <text fg={fg}>{message}</text>
    </box>
  );
}
