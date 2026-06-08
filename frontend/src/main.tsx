#!/usr/bin/env node
import React from "react";
import { render } from "ink";
import { App } from "./app/App.js";
import { createBridgeClient } from "./rpc/client.js";
import type { AppMode } from "./app/routes.js";

const [modeArg, bridgeFlag, bridgeCommand] = process.argv.slice(2);
const mode = modeArg === "admin" ? "admin" : "config";
if (bridgeFlag !== "--bridge-command" || !bridgeCommand) {
  throw new Error("Usage: main.js <admin|config> --bridge-command <command>");
}

const client = createBridgeClient(bridgeCommand);
render(<App mode={mode as AppMode} client={client} />);
