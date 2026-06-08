import React, { useEffect, useState } from "react";
import { Text } from "ink";
import { ConfigBootstrapSchema, type ConfigBootstrap } from "../rpc/schema.js";
import { ConfigScreen } from "../config/ConfigScreen.js";
import type { JsonRpcClient } from "../rpc/client.js";
import type { AppMode } from "./routes.js";

type Props = {
  mode: AppMode;
  client: JsonRpcClient;
};

export function App({ mode, client }: Props) {
  const [configInitial, setConfigInitial] = useState<ConfigBootstrap | null>(
    null,
  );
  const [error, setError] = useState("");

  useEffect(() => {
    if (mode === "config") {
      client
        .request("config.bootstrap", {})
        .then((payload) =>
          setConfigInitial(ConfigBootstrapSchema.parse(payload)),
        )
        .catch((err: Error) => setError(err.message));
    }
  }, [client, mode]);

  if (mode === "admin") {
    return <Text color="yellow">Admin Ink screen is not available yet</Text>;
  }
  if (error) {
    return <Text color="red">{error}</Text>;
  }
  if (mode === "config" && configInitial) {
    return <ConfigScreen initial={configInitial} client={client} />;
  }
  return <Text>Loading {mode}...</Text>;
}
