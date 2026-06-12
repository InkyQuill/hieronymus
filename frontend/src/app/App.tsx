import React, { useEffect, useState } from "react";
import {
  AdminBootstrapSchema,
  ConfigBootstrapSchema,
  type AdminBootstrap,
  type ConfigBootstrap,
} from "../rpc/schema.js";
import { AdminScreen } from "../admin/AdminScreen.js";
import { ConfigScreen } from "../config/ConfigScreen.js";
import type { RpcClient } from "../rpc/client.js";
import type { AppMode } from "./routes.js";
import { Spinner } from "../ui/Spinner.js";

type Props = {
  mode: AppMode;
  client: RpcClient;
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
        .catch((err: unknown) => {
          if (active) {
            setError(errorMessage(err));
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
        .catch((err: unknown) => {
          if (active) {
            setError(errorMessage(err));
          }
        });
    }

    return () => {
      active = false;
    };
  }, [client, mode]);

  if (error) {
    return <text fg="red">{error}</text>;
  }
  if (mode === "admin" && adminInitial) {
    return <AdminScreen initial={adminInitial} client={client} />;
  }
  if (mode === "config" && configInitial) {
    return <ConfigScreen initial={configInitial} client={client} />;
  }
  return (
    <box padding={1}>
      <Spinner name="helix" label={`Loading ${mode}...`} fg="cyan" />
    </box>
  );
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
