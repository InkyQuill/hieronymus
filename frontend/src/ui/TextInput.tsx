import React from "react";

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
  return (
    <input
      value={value}
      onChange={onChange}
      onSubmit={onSubmit}
      placeholder={placeholder}
      focused={focus}
    />
  );
}
