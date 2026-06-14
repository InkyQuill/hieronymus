import React, { useRef } from "react";
import type { TextareaRenderable } from "@opentui/core";

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

type TextAreaInputProps = TextInputProps & {
  width?: number;
  height?: number;
};

export function TextAreaInput({
  value,
  onChange,
  onSubmit,
  placeholder = "",
  focus = true,
  width = 46,
  height = 6,
}: TextAreaInputProps) {
  const textareaRef = useRef<TextareaRenderable>(null);

  return (
    <textarea
      ref={textareaRef}
      initialValue={value}
      onContentChange={() => {
        onChange(textareaRef.current?.plainText ?? value);
      }}
      onSubmit={onSubmit}
      placeholder={placeholder}
      focused={focus}
      width={width}
      height={height}
      wrapMode="word"
      // OpenTUI 0.4.0 textarea exposes scrolling behavior but no visible
      // scrollbar option; keep bounded dimensions until that API exists.
      scrollMargin={2}
      cursorColor="cyan"
      focusedTextColor="white"
      focusedBackgroundColor="transparent"
    />
  );
}
