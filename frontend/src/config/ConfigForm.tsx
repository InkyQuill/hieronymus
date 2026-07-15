import React from "react";
import type { ConfigFormField, ConfigFormGroup } from "../rpc/schema.js";
import { theme } from "../ui/theme.js";

type RenderedField = ConfigFormField & { value: string };

type RenderedGroup = {
  group: ConfigFormGroup;
  fields: Array<{ field: RenderedField; index: number }>;
};

type ConfigFormProps = {
  groups: ConfigFormGroup[];
  fields: ConfigFormField[];
  formValues: {
    provider: Record<string, string>;
    providerCatalog: Record<string, string>;
    workflows: Record<string, string>;
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
  groups,
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
  const providerCatalog = formValues.providerCatalog;
  const workflows = formValues.workflows;
  const dreaming = formValues.dreaming;
  const ingest = formValues.ingest;
  const release = formValues.release;
  const boundedWidth = Math.max(20, width);

  const renderedFields = fields.map((field): RenderedField => {
    let value = "";
    if (field.key.startsWith("provider.")) {
      value = provider[field.key.slice(9)] || "";
    } else if (field.key.startsWith("provider_catalog.")) {
      value = providerCatalog[field.key.slice(17)] || "";
    } else if (field.key.startsWith("workflows.")) {
      value = workflows[field.key.slice(10)] || "";
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

  const renderedGroups = groupRenderedFields(groups, renderedFields);
  const visibleIndexes = visibleIndexSet(
    renderedFields.length,
    focusedFieldIndex,
    visibleRows,
  );
  const focusedGroup = renderedFields[focusedFieldIndex]?.group ?? "";

  return (
    <box flexDirection="column" width={boundedWidth}>
      <text>Configuration settings</text>
      <box flexDirection="column" marginTop={1}>
        {renderedGroups.map(({ group, fields: groupFields }) => {
          if (visibleRows !== undefined && group.id !== focusedGroup) {
            return null;
          }
          const visibleGroupFields = groupFields.filter(({ index }) =>
            visibleIndexes.has(index),
          );
          const activeField = groupFields.find(
            ({ index }) => focusedFieldIndex === index,
          )?.field;
          const isGroupActive = focused && activeField !== undefined;

          if (visibleGroupFields.length === 0 && activeField === undefined) {
            return null;
          }

          return (
            <box
              key={group.id}
              flexDirection="column"
              width={boundedWidth}
              marginTop={1}
              borderStyle="rounded"
              borderColor={
                isGroupActive ? theme.accentPrimary : theme.accentMuted
              }
              paddingX={1}
            >
              <box flexDirection="row">
                <text
                  fg={isGroupActive ? theme.accentPrimary : theme.accentMuted}
                >
                  {displayGroupLabel(group)}
                </text>
                {configFileLabel(group) ? (
                  <text fg={theme.accentMuted}>
                    {" "}
                    [{configFileLabel(group)}]
                  </text>
                ) : null}
              </box>
              {/* Windowed layouts keep only the active field hint to preserve footer space. */}
              {group.description && visibleRows === undefined ? (
                <text fg={theme.accentMuted}>{group.description}</text>
              ) : null}

              {visibleGroupFields.map(({ field, index }) => {
                const isFieldFocused = focused && focusedFieldIndex === index;
                const labelColor = isFieldFocused
                  ? theme.accentPrimary
                  : theme.accentMuted;
                const fieldInputWidth = Math.max(
                  12,
                  boundedWidth - field.label.length - 8,
                );

                return (
                  <box key={field.key} flexDirection="row" width="100%">
                    <text fg={labelColor}>
                      {isFieldFocused ? "> " : "  "}
                      {field.label}:{" "}
                    </text>

                    {field.type === "toggle" || field.type === "choice" ? (
                      <box flexDirection="row" flexGrow={1}>
                        {isFieldFocused && isEditing ? (
                          <box flexDirection="row" flexWrap="wrap" flexGrow={1}>
                            {(field.choices.length
                              ? field.choices
                              : ["yes", "no"]
                            ).map((choice) => (
                              <text
                                key={choice}
                                fg={
                                  field.value === choice
                                    ? theme.accentPrimary
                                    : theme.accentMuted
                                }
                              >
                                [{choice}]{" "}
                              </text>
                            ))}
                          </box>
                        ) : (
                          <text
                            fg={
                              isFieldFocused ? theme.accentPrimary : undefined
                            }
                          >
                            {field.value}
                          </text>
                        )}
                      </box>
                    ) : (
                      <input
                        value={field.value}
                        onInput={(val) => onFieldChange(field.key, val)}
                        onSubmit={() => onSubmitField()}
                        focused={isFieldFocused && isEditing}
                        placeholder={field.placeholder}
                        width={fieldInputWidth}
                      />
                    )}
                  </box>
                );
              })}

              {isGroupActive && activeField?.hint ? (
                <text fg={theme.accentPrimary} marginTop={1}>
                  {activeField.hint}
                </text>
              ) : null}
            </box>
          );
        })}
      </box>
    </box>
  );
}

function configFileLabel(group: ConfigFormGroup): string {
  if (group.section === "providers" || group.section === "provider_catalog") {
    return "provider.conf";
  }
  if (group.section === "workflows") {
    return "dream.conf";
  }
  if (group.section === "dream") {
    return "dream.conf";
  }
  if (group.section === "ingest") {
    return "ingest.conf";
  }
  if (group.section === "release") {
    return "release.conf";
  }
  return group.section || "";
}

function displayGroupLabel(group: ConfigFormGroup): string {
  if (group.id === "provider" || group.id === "provider_catalog") {
    return "Providers";
  }
  if (group.id === "workflows") {
    return "Workflows";
  }
  if (group.id === "ingest") {
    return "Ingestion";
  }
  if (group.id === "release") {
    return "Updates";
  }
  return group.label;
}

function groupRenderedFields(
  groups: ConfigFormGroup[],
  fields: RenderedField[],
): RenderedGroup[] {
  const fieldsByGroup = new Map<
    string,
    Array<{ field: RenderedField; index: number }>
  >();
  const knownGroupIds = new Set(groups.map((group) => group.id));
  const orphanFields: Array<{ field: RenderedField; index: number }> = [];

  fields.forEach((field, index) => {
    if (!knownGroupIds.has(field.group)) {
      orphanFields.push({ field, index });
      return;
    }

    const groupFields = fieldsByGroup.get(field.group) || [];
    groupFields.push({ field, index });
    fieldsByGroup.set(field.group, groupFields);
  });

  const renderedGroups: RenderedGroup[] = groups
    .map((group) => ({
      group,
      fields: fieldsByGroup.get(group.id) || [],
    }))
    .filter((renderedGroup) => renderedGroup.fields.length > 0);

  if (orphanFields.length > 0) {
    renderedGroups.push({
      group: {
        id: "other",
        section: "",
        label: "Other",
        description: "Additional configuration fields.",
      },
      fields: orphanFields,
    });
  }

  return renderedGroups;
}

function visibleIndexSet(
  fieldCount: number,
  focusedFieldIndex: number,
  visibleRows: number | undefined,
): Set<number> {
  if (visibleRows !== undefined) {
    const fieldWindow = getVisibleFieldWindow(
      fieldCount,
      focusedFieldIndex,
      visibleRows,
    );
    const visibleIndexes = new Set<number>();

    for (let index = fieldWindow.start; index < fieldWindow.end; index += 1) {
      visibleIndexes.add(index);
    }

    return visibleIndexes;
  }

  const fieldWindow = getVisibleFieldWindow(
    fieldCount,
    focusedFieldIndex,
    visibleRows,
  );
  const visibleIndexes = new Set<number>();

  for (let index = fieldWindow.start; index < fieldWindow.end; index += 1) {
    visibleIndexes.add(index);
  }

  return visibleIndexes;
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
