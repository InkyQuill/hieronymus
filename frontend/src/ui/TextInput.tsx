import React from "react";
import { Text, useInput, useStdin } from "ink";

type TextInputProps = {
  value: string;
  onChange: (value: string) => void;
  onSubmit?: () => void;
  placeholder?: string;
  focus?: boolean;
};

export function TextInput({
  value,
  onChange,
  onSubmit,
  placeholder = "",
  focus = true,
}: TextInputProps) {
  const { stdin, isRawModeSupported } = useStdin();
  const canUseInkInput = Boolean(
    isRawModeSupported &&
      typeof stdin.ref === "function" &&
      typeof stdin.unref === "function",
  );

  useInput(
    (input, key) => {
      if (!focus) {
        return;
      }
      if (key.return) {
        onSubmit?.();
        return;
      }
      if (key.backspace || key.delete) {
        onChange(value.slice(0, -1));
        return;
      }
      // Capture standard printable characters, excluding control and meta combos
      if (input && input.length === 1 && !key.ctrl && !key.meta) {
        onChange(value + input);
      }
    },
    { isActive: focus && canUseInkInput },
  );

  if (!value && placeholder) {
    return (
      <Text dimColor>
        {placeholder}
        {focus ? <Text>█</Text> : null}
      </Text>
    );
  }

  return (
    <Text color={focus ? "cyan" : undefined}>
      {value}
      {focus ? <Text>█</Text> : null}
    </Text>
  );
}
