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
    let active = true;
    setError("");

    if (mode === "admin") {
      setAdminInitial(null);
      setConfigInitial(null);
      client
        .request("admin.bootstrap", {})
        .then((payload) => {
          if (active) {
            setAdminInitial(AdminBootstrapSchema.parse(payload));
          }
        })
        .catch((err: Error) => {
          if (active) {
            setError(err.message);
          }
        });
    }

    if (mode === "config") {
      setAdminInitial(null);
      setConfigInitial(null);
      client
        .request("config.bootstrap", {})
        .then((payload) => {
          if (active) {
            setConfigInitial(ConfigBootstrapSchema.parse(payload));
          }
        })
        .catch((err: Error) => {
          if (active) {
            setError(err.message);
          }
        });
    }

    return () => {
      active = false;
    };
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
