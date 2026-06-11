#!/usr/bin/env bun
import React from "react";
import { createCliRenderer } from "@opentui/core";
import { createRoot } from "@opentui/react";
import { App } from "./app/App.js";
import { createBridgeClient } from "./rpc/client.js";
import type { AppMode } from "./app/routes.js";

const [modeArg, ...args] = process.argv.slice(2);

function parseBridgeArgs(argv: string[]) {
  let bridgeCommand: string | undefined;
  const bridgeArgs: string[] = [];
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--bridge-command") {
      bridgeCommand = argv[index + 1];
      index += 1;
      continue;
    }
    if (arg === "--bridge-arg") {
      const value = argv[index + 1];
      if (value !== undefined) {
        bridgeArgs.push(value);
      }
      index += 1;
      continue;
    }
    return { bridgeCommand: undefined, bridgeArgs: [] };
  }
  return { bridgeCommand, bridgeArgs };
}

const { bridgeCommand, bridgeArgs } = parseBridgeArgs(args);

async function main() {
  if (
    (modeArg !== "admin" && modeArg !== "config") ||
    !bridgeCommand
  ) {
    console.error(
      "Usage: main.js <admin|config> --bridge-command <command> [--bridge-arg <arg>...]",
    );
    process.exitCode = 1;
    return;
  }

  const client = createBridgeClient(bridgeCommand, bridgeArgs);
  const renderer = await createCliRenderer({
    exitOnCtrlC: true,
  });
  createRoot(renderer).render(<App mode={modeArg as AppMode} client={client} />);
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
