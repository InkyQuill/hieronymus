#!/usr/bin/env bun
import { appendFileSync } from "node:fs";
import { resolve } from "node:path";
import { parseArgs } from "node:util";
import { desktopTarget } from "./desktop-targets";
import { stage } from "./release-assets";
try {
  const { values } = parseArgs({
    args: Bun.argv.slice(2),
    options: {
      root: { type: "string", default: resolve(import.meta.dir, "..") },
      target: { type: "string", default: "x86_64-unknown-linux-gnu" },
      "runtime-archive": { type: "string" },
      "model-dir": { type: "string" },
      "github-env": { type: "string" },
    },
  });
  const valuesToExport = await stage(
    values.root!,
    desktopTarget(values.target!).target,
    {
      runtimeArchive: values["runtime-archive"],
      modelDirectory: values["model-dir"],
    },
  );
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
} catch (error) {
  // Do not print network exceptions: redirect URLs may include signed queries.
  console.error(
    error instanceof Error && error.message.startsWith("Intel ONNX")
      ? "Intel ONNX 1.28.0 staging refused: run the pinned native source-build route and review measured output/provenance before promoting runtime pins"
      : "Release asset staging refused: acquisition or verification failed",
  );
  process.exitCode = 1;
}
