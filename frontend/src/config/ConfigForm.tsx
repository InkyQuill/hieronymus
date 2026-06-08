import React from "react";
import { Box, Text } from "ink";
import type { ConfigBootstrap } from "../rpc/schema.js";

type Props = {
  payload: ConfigBootstrap;
};

export function ConfigForm({ payload }: Props) {
  const provider = payload.form_values.provider;
  const dreaming = payload.form_values.dreaming;
  return (
    <Box flexDirection="column">
      <Text bold>Provider</Text>
      <Text>model: {provider.model || "-"}</Text>
      <Text>api_key_env: {provider.api_key_env || "-"}</Text>
      <Text>api_path: {provider.api_path || "-"}</Text>
      <Text>timeout_seconds: {provider.timeout_seconds || "-"}</Text>
      <Text bold>Dreaming</Text>
      <Text>autostart_enabled: {dreaming.autostart_enabled || "no"}</Text>
      <Text>min_interval_minutes: {dreaming.min_interval_minutes || "-"}</Text>
      <Text>
        new_short_term_memory_threshold:{" "}
        {dreaming.new_short_term_memory_threshold || "-"}
      </Text>
      <Text>
        max_cycles_per_autostart: {dreaming.max_cycles_per_autostart || "-"}
      </Text>
    </Box>
  );
}
