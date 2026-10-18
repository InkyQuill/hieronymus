#!/usr/bin/env bun
import { appendFileSync } from "node:fs";
import { resolve } from "node:path";
import { stage } from "./release-assets";
import { acquireReviewedRuntime } from "./reviewed-runtime";
import { desktopTarget } from "./desktop-targets";
const target = desktopTarget(Bun.argv[2] ?? "").target;
const reviewed = await acquireReviewedRuntime(resolve("."), target);
const env = await stage(resolve("."), target, {
  skipModel: true,
  ...(reviewed ? { runtimeArchive: reviewed.archive } : {}),
});
if (!process.env.GITHUB_ENV) throw new Error("GITHUB_ENV required");
appendFileSync(
  process.env.GITHUB_ENV,
  Object.entries({
    ...env,
    ...(reviewed
      ? {
          HIERO_REVIEWED_RUNTIME_ARCHIVE: reviewed.archive,
          HIERO_REVIEWED_RUNTIME_RECEIPT: reviewed.receipt,
        }
      : {}),
  })
    .map(([k, v]) => {
      if (/[\r\n]/.test(v)) throw new Error("invalid environment path");
      return `${k}=${v}\n`;
    })
    .join(""),
);
