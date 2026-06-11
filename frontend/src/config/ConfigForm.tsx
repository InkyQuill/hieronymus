import React from "react";
import { Box, Text } from "ink";
import { TextInput } from "../ui/TextInput.js";

type ConfigFormProps = {
  formValues: {
    provider: Record<string, string>;
    dreaming: Record<string, string>;
  };
  focusedFieldIndex: number;
  isEditing: boolean;
  focused?: boolean;
  onFieldChange: (key: string, value: string) => void;
  onSubmitField: () => void;
};

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

  const fields = [
    {
      label: "Model",
      value: provider.model || "",
      key: "provider.model",
      placeholder: "e.g. gpt-4.1-mini",
      type: "text",
    },
    {
      label: "API Key Env",
      value: provider.api_key_env || "",
      key: "provider.api_key_env",
      placeholder: "e.g. OPENAI_API_KEY",
      type: "text",
    },
    {
      label: "API Path",
      value: provider.api_path || "",
      key: "provider.api_path",
      placeholder: "e.g. https://api.openai.com/v1",
      type: "text",
    },
    {
      label: "Timeout (seconds)",
      value: provider.timeout_seconds || "",
      key: "provider.timeout_seconds",
      placeholder: "e.g. 30",
      type: "text",
    },
    {
      label: "Autostart Enabled",
      value: dreaming.autostart_enabled || "no",
      key: "dreaming.autostart_enabled",
      type: "toggle",
    },
    {
      label: "Min Interval (minutes)",
      value: dreaming.min_interval_minutes || "",
      key: "dreaming.min_interval_minutes",
      placeholder: "e.g. 30",
      type: "text",
    },
    {
      label: "New Memory Threshold",
      value: dreaming.new_short_term_memory_threshold || "",
      key: "dreaming.new_short_term_memory_threshold",
      placeholder: "e.g. 25",
      type: "text",
    },
    {
      label: "Max Cycles Per Autostart",
      value: dreaming.max_cycles_per_autostart || "",
      key: "dreaming.max_cycles_per_autostart",
      placeholder: "e.g. 1",
      type: "text",
    },
  ];

  return (
    <Box flexDirection="column" width={68}>
      <Text bold color={focused ? "cyan" : undefined}>
        Dreaming settings
      </Text>
      <Box flexDirection="column" marginTop={1}>
        {fields.map((field, index) => {
          const isFieldFocused = focused && focusedFieldIndex === index;
          const labelColor = isFieldFocused ? "cyan" : "gray";

          return (
            <Box key={field.key} flexDirection="row" marginTop={index === 4 ? 1 : 0}>
              <Text color={labelColor} bold={isFieldFocused}>
                {isFieldFocused ? ">" : " "} {field.label}:{" "}
              </Text>

              {field.type === "toggle" ? (
                <Box flexDirection="row">
                  {isFieldFocused && isEditing ? (
                    <Box flexDirection="row">
                      <Text
                        color={field.value === "yes" ? "cyan" : "gray"}
                        bold={field.value === "yes"}
                      >
                        [Yes]{" "}
                      </Text>
                      <Text
                        color={field.value === "no" ? "cyan" : "gray"}
                        bold={field.value === "no"}
                      >
                        [No]
                      </Text>
                    </Box>
                  ) : (
                    <Text color={isFieldFocused ? "cyan" : undefined}>
                      {field.value}
                    </Text>
                  )}
                </Box>
              ) : (
                <TextInput
                  value={field.value}
                  onChange={(val) => onFieldChange(field.key, val)}
                  focus={isFieldFocused && isEditing}
                  placeholder={field.placeholder}
                  onSubmit={onSubmitField}
                />
              )}
            </Box>
          );
        })}
      </Box>
    </Box>
  );
}
