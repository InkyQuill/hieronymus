import React from "react";
import type { ConfigBootstrap } from "../rpc/schema.js";

type ConfigFormProps = {
  formValues: {
    provider: Record<string, string>;
    dreaming: Record<string, string>;
    release: Record<string, string>;
  };
  focusedFieldIndex: number;
  isEditing: boolean;
  focused?: boolean;
  onFieldChange: (key: string, value: string) => void;
  onSubmitField: () => void;
};

export type FieldDefinition = {
  label: string;
  key: string;
  placeholder?: string;
  type: "text" | "toggle" | "choice";
  choices?: string[];
};

export const fieldDefinitions: FieldDefinition[] = [
  {
    label: "Model",
    key: "provider.model",
    placeholder: "e.g. gpt-4.1-mini",
    type: "text",
  },
  {
    label: "API Key Env",
    key: "provider.api_key_env",
    placeholder: "e.g. OPENAI_API_KEY",
    type: "text",
  },
  {
    label: "API Path",
    key: "provider.api_path",
    placeholder: "e.g. https://api.openai.com/v1",
    type: "text",
  },
  {
    label: "Timeout (seconds)",
    key: "provider.timeout_seconds",
    placeholder: "e.g. 30",
    type: "text",
  },
  {
    label: "Autostart Enabled",
    key: "dreaming.autostart_enabled",
    type: "toggle",
  },
  {
    label: "Update Channel",
    key: "release.update_channel",
    type: "choice",
    choices: ["stable", "dev"],
  },
  {
    label: "Min Interval (minutes)",
    key: "dreaming.min_interval_minutes",
    placeholder: "e.g. 30",
    type: "text",
  },
  {
    label: "New Memory Threshold",
    key: "dreaming.new_short_term_memory_threshold",
    placeholder: "e.g. 25",
    type: "text",
  },
  {
    label: "Max Cycles Per Autostart",
    key: "dreaming.max_cycles_per_autostart",
    placeholder: "e.g. 1",
    type: "text",
  },
];

export function ConfigForm({
  formValues,
  focusedFieldIndex,
  isEditing,
  focused = true,
  onFieldChange,
  onSubmitField,
}: ConfigFormProps) {
  const provider = formValues.provider;
  const dreaming = formValues.dreaming;
  const release = formValues.release;

  const fields = fieldDefinitions.map((field) => {
    let value = "";
    if (field.key.startsWith("provider.")) {
      value = provider[field.key.slice(9)] || "";
    } else if (field.key.startsWith("dreaming.")) {
      value = dreaming[field.key.slice(9)] || "";
    } else if (field.key.startsWith("release.")) {
      value = release[field.key.slice(8)] || "";
    }
    if (field.key === "dreaming.autostart_enabled" && !value) {
      value = "no";
    }
    return {
      ...field,
      value,
    };
  });

  const autostartIndex = fieldDefinitions.findIndex(
    (f) => f.key === "dreaming.autostart_enabled",
  );

  return (
    <box flexDirection="column" width={68}>
      <text>Dreaming settings</text>
      <box flexDirection="column" marginTop={1}>
        {fields.map((field, index) => {
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
                      {(field.choices || ["yes", "no"]).map((choice) => (
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
                  onChange={(val) => onFieldChange(field.key, val)}
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
