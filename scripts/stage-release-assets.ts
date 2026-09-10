#!/usr/bin/env bun
import { appendFileSync } from "node:fs";
import { resolve } from "node:path";
import { parseArgs } from "node:util";
import { stage } from "./release-assets";
try {
  const { values } = parseArgs({
    args: Bun.argv.slice(2),
    options: {
      root: { type: "string", default: resolve(import.meta.dir, "..") },
      "github-env": { type: "string" },
    },
  });
  const valuesToExport = await stage(values.root!);
  if (Object.values(valuesToExport).some((value) => /[\r\n]/.test(value)))
    throw new Error("invalid environment path");
  if (values["github-env"])
    appendFileSync(
      values["github-env"],
      Object.entries(valuesToExport)
        .map(([key, value]) => `${key}=${value}\n`)
        .join(""),
    );
  console.log(JSON.stringify(valuesToExport));
} catch {
  // Do not print network exceptions: redirect URLs may include signed queries.
  console.error(
    "Release asset staging refused: acquisition or verification failed",
  );
  process.exitCode = 1;
}
