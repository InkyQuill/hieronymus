#!/usr/bin/env bun
import React from "react";
import { createCliRenderer } from "@opentui/core";
import { createRoot } from "@opentui/react";
import { App } from "./app/App.js";
import { createBridgeClient } from "./rpc/client.js";
import type { AppMode } from "./app/routes.js";

const [modeArg, ...args] = process.argv.slice(2);
const usage =
  "Usage: main.js <admin|config> --bridge-command <command> [--bridge-arg <arg>...]";

function parseBridgeArgs(argv: string[]) {
  let bridgeCommand: string | undefined;
  const bridgeArgs: string[] = [];
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--bridge-command") {
      const value = argv[index + 1];
      if (value === undefined) {
        throw new Error("--bridge-command requires a value");
      }
      bridgeCommand = value;
      index += 1;
      continue;
    }
    if (arg === "--bridge-arg") {
      const value = argv[index + 1];
      if (value === undefined) {
        throw new Error("--bridge-arg requires a value");
      }
      bridgeArgs.push(value);
      index += 1;
      continue;
    }
    throw new Error(`Unknown argument: ${arg}`);
  }
  return { bridgeCommand, bridgeArgs };
}

async function main() {
  let bridgeCommand: string | undefined;
  let bridgeArgs: string[];
  try {
    ({ bridgeCommand, bridgeArgs } = parseBridgeArgs(args));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    console.error(usage);
    process.exitCode = 1;
    return;
  }

  if ((modeArg !== "admin" && modeArg !== "config") || !bridgeCommand) {
    console.error(usage);
    process.exitCode = 1;
    return;
  }

  const client = createBridgeClient(bridgeCommand, bridgeArgs);
  const renderer = await createCliRenderer({
    exitOnCtrlC: true,
  });
  createRoot(renderer).render(
    <App mode={modeArg as AppMode} client={client} />,
  );
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
