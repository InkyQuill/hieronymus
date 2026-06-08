import React, { useEffect, useState } from "react";
import { Text } from "ink";
import {
  AdminBootstrapSchema,
  ConfigBootstrapSchema,
  type AdminBootstrap,
  type ConfigBootstrap,
} from "../rpc/schema.js";
import { AdminScreen } from "../admin/AdminScreen.js";
import { ConfigScreen } from "../config/ConfigScreen.js";
import type { JsonRpcClient } from "../rpc/client.js";
import type { AppMode } from "./routes.js";

type Props = {
  mode: AppMode;
  client: JsonRpcClient;
};

export function App({ mode, client }: Props) {
  const [adminInitial, setAdminInitial] = useState<AdminBootstrap | null>(null);
  const [configInitial, setConfigInitial] = useState<ConfigBootstrap | null>(
    null,
  );
  const [error, setError] = useState("");

  useEffect(() => {
    if (mode === "admin") {
      client
        .request("admin.bootstrap", {})
        .then((payload) => setAdminInitial(AdminBootstrapSchema.parse(payload)))
        .catch((err: Error) => setError(err.message));
    }

    if (mode === "config") {
      client
        .request("config.bootstrap", {})
        .then((payload) =>
          setConfigInitial(ConfigBootstrapSchema.parse(payload)),
        )
        .catch((err: Error) => setError(err.message));
    }
  }, [client, mode]);

  if (error) {
    return <Text color="red">{error}</Text>;
  }
  if (mode === "admin" && adminInitial) {
    return <AdminScreen initial={adminInitial} client={client} />;
  }
  if (mode === "config" && configInitial) {
    return <ConfigScreen initial={configInitial} client={client} />;
  }
  return <Text>Loading {mode}...</Text>;
}
