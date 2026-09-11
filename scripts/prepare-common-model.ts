#!/usr/bin/env bun
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { stage } from "./release-assets";
import { packageCommonModel } from "./shared-model";
if (
  process.platform !== "linux" ||
  process.arch !== "x64" ||
  Bun.version !== "1.4.0"
)
  throw new Error("canonical producer requires Linux x64 and Bun 1.4.0");
const env = await stage(resolve("."));
const out = resolve("target/common-model");
const model = await packageCommonModel(env.HIERO_RELEASE_MODEL_DIR, out);
writeFileSync(
  resolve(out, "common-model.json"),
  JSON.stringify(model, null, 2) + "\n",
  { flag: "wx" },
);
