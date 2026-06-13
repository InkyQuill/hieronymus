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
  onFieldChange: (key: string, value: string) => void;
  onSubmitField: () => void;
};

export function ConfigForm({
  fields,
  formValues,
  focusedFieldIndex,
  isEditing,
  focused = true,
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

  return (
    <box flexDirection="column" width={68}>
      <text>Dreaming settings</text>
      <box flexDirection="column" marginTop={1}>
        {renderedFields.map((field, index) => {
          const isFieldFocused = focused && focusedFieldIndex === index;
          const labelColor = isFieldFocused ? "cyan" : "gray";

          return (
            <box
              key={field.key}
              flexDirection="row"
              marginTop={index === autostartIndex ? 1 : 0}
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
                  onSubmit={onSubmitField}
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
