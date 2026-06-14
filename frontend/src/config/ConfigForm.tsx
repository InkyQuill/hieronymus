import React from "react";
import type { ConfigFormField } from "../rpc/schema.js";

type ConfigFormProps = {
  fields: ConfigFormField[];
  formValues: {
    provider: Record<string, string>;
    dreaming: Record<string, string>;
    ingest: Record<string, string>;
    release: Record<string, string>;
  };
  focusedFieldIndex: number;
  isEditing: boolean;
  focused?: boolean;
  width?: number;
  visibleRows?: number;
  onFieldChange: (key: string, value: string) => void;
  onSubmitField: () => void;
};

export function ConfigForm({
  fields,
  formValues,
  focusedFieldIndex,
  isEditing,
  focused = true,
  width = 68,
  visibleRows,
  onFieldChange,
  onSubmitField,
}: ConfigFormProps) {
  const provider = formValues.provider;
  const dreaming = formValues.dreaming;
  const ingest = formValues.ingest;
  const release = formValues.release;

  const renderedFields = fields.map((field) => {
    let value = "";
    if (field.key.startsWith("provider.")) {
      value = provider[field.key.slice(9)] || "";
    } else if (field.key.startsWith("dreaming.")) {
      value = dreaming[field.key.slice(9)] || "";
    } else if (field.key.startsWith("ingest.")) {
      value = ingest[field.key.slice(7)] || "";
    } else if (field.key.startsWith("release.")) {
      value = release[field.key.slice(8)] || "";
    }
    if ((field.type === "toggle" || field.type === "choice") && !value) {
      value = field.default || field.choices[0] || "";
    }
    return {
      ...field,
      value,
    };
  });

  const autostartIndex = renderedFields.findIndex(
    (f) => f.key === "dreaming.autostart_enabled",
  );
  const fieldWindow = getVisibleFieldWindow(
    renderedFields.length,
    focusedFieldIndex,
    visibleRows,
  );
  const visibleFields = renderedFields.slice(
    fieldWindow.start,
    fieldWindow.end,
  );

  return (
    <box flexDirection="column" width={width}>
      <text>Dreaming settings</text>
      <box flexDirection="column" marginTop={1}>
        {visibleFields.map((field, visibleIndex) => {
          const index = fieldWindow.start + visibleIndex;
          const isFieldFocused = focused && focusedFieldIndex === index;
          const labelColor = isFieldFocused ? "cyan" : "gray";

          return (
            <box
              key={field.key}
              flexDirection="row"
              marginTop={
                visibleRows === undefined && index === autostartIndex ? 1 : 0
              }
            >
              <text fg={labelColor}>
                {isFieldFocused ? "> " : "  "}
                {field.label}:{" "}
              </text>

              {field.type === "toggle" || field.type === "choice" ? (
                <box flexDirection="row">
                  {isFieldFocused && isEditing ? (
                    <box flexDirection="row">
                      {(field.choices.length
                        ? field.choices
                        : ["yes", "no"]
                      ).map((choice) => (
                        <text
                          key={choice}
                          fg={field.value === choice ? "cyan" : "gray"}
                        >
                          [{choice}]{" "}
                        </text>
                      ))}
                    </box>
                  ) : (
                    <text fg={isFieldFocused ? "cyan" : undefined}>
                      {field.value}
                    </text>
                  )}
                </box>
              ) : isFieldFocused && isEditing ? (
                <input
                  value={field.value}
                  onInput={(val) => onFieldChange(field.key, val)}
                  onSubmit={() => onSubmitField()}
                  focused={true}
                  placeholder={field.placeholder}
                />
              ) : (
                <text fg={isFieldFocused ? "cyan" : undefined}>
                  {field.value || field.placeholder || " "}
                </text>
              )}
            </box>
          );
        })}
      </box>
    </box>
  );
}

function getVisibleFieldWindow(
  fieldCount: number,
  focusedFieldIndex: number,
  visibleRows: number | undefined,
): { start: number; end: number } {
  if (visibleRows === undefined || visibleRows >= fieldCount) {
    return { start: 0, end: fieldCount };
  }

  const windowSize = Math.max(0, visibleRows);
  if (windowSize === 0) {
    return { start: 0, end: 0 };
  }

  const focusedIndex = Math.min(
    Math.max(focusedFieldIndex, 0),
    Math.max(fieldCount - 1, 0),
  );
  const preferredStart = focusedIndex - Math.floor(windowSize / 2);
  const maxStart = Math.max(fieldCount - windowSize, 0);
  const start = Math.min(Math.max(preferredStart, 0), maxStart);

  return { start, end: Math.min(start + windowSize, fieldCount) };
}
