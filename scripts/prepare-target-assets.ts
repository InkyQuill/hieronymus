#!/usr/bin/env bun
import { appendFileSync, mkdirSync, readFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { stage, verify, extractTar } from "./release-assets";
import { gunzipSync } from "node:zlib";
import {
  desktopTarget,
  parseReleaseV2,
  platformArtifactName,
} from "./desktop-targets";
const target = desktopTarget(Bun.argv[2] ?? "").target;
const common = resolve("target/common-model");
const model = JSON.parse(
  readFileSync(join(common, "common-model.json"), "utf8"),
);
parseReleaseV2(
  JSON.stringify({
    format_version: 2,
    version: "0.0.0",
    target,
    channel: "dev",
    platform: {
      archive: platformArtifactName("0.0.0", target),
      sha256: "0".repeat(64),
    },
    model,
    signature: null,
  }),
  target,
);
await verify(join(common, model.archive), model.sha256);
const extracted = resolve("target/common-expanded");
mkdirSync(extracted, { recursive: true });
extractTar(
  gunzipSync(readFileSync(join(common, model.archive)), {
    maxOutputLength: 1024 * 1024 * 1024,
  }),
  extracted,
  "models",
  1024 * 1024 * 1024,
);
const env = await stage(resolve("."), target, {
  modelDirectory: join(extracted, "models/minilm"),
});
if (!process.env.GITHUB_ENV) throw new Error("GITHUB_ENV required");
appendFileSync(
  process.env.GITHUB_ENV,
  Object.entries({ ...env, HIERO_COMMON_MODEL_DIR: common })
    .map(([k, v]) => {
      if (/[\r\n]/.test(v)) throw new Error("invalid environment path");
      return `${k}=${v}\n`;
    })
    .join(""),
);
