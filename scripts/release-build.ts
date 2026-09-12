#!/usr/bin/env bun
/** Native target producer. No publication and no per-target model recompression. */
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { parseArgs } from "node:util";
import { desktopTarget } from "./desktop-targets";
import { packageDesktop } from "./desktop-package";
function run(args: string[]) {
  const result = Bun.spawnSync(args, { stdout: "inherit", stderr: "inherit" });
  if (result.exitCode !== 0)
    throw new Error(`${args[0]} failed (${result.exitCode})`);
}
const { values } = parseArgs({
  args: Bun.argv.slice(2),
  options: {
    target: { type: "string", default: "x86_64-unknown-linux-gnu" },
    out: { type: "string", default: "target/release-dist" },
    "assets-only": { type: "boolean" },
    "dry-run": { type: "boolean" },
  },
});
const target = desktopTarget(values.target!).target;
if (values["dry-run"]) {
  console.log(
    `Build frontend, embedded CLI and desktop helper for ${target}; package strict release-${target}.json plus existing canonical common-model archive; no publication`,
  );
} else {
  if (Bun.version !== "1.4.0") throw new Error("release requires Bun 1.4.0");
  run(["bun", "install", "--cwd", "frontend", "--frozen-lockfile"]);
  run(["bun", "run", "--cwd", "frontend", "build"]);
  if (values["assets-only"])
    run([
      "cargo",
      "check",
      "--locked",
      "-p",
      "hiero",
      "--features",
      "console-embed",
    ]);
  else {
    if (
      !process.env.HIERO_COMMON_MODEL_DIR ||
      !process.env.HIERO_RELEASE_RUNTIME_DIR
    )
      throw new Error(
        "HIERO_COMMON_MODEL_DIR and HIERO_RELEASE_RUNTIME_DIR required; produce shared archive once and stage exact pinned runtime first",
      );
    run([
      "cargo",
      "test",
      "--locked",
      "-p",
      "hiero",
      "--features",
      "console-embed",
      "--test",
      "console_embed",
    ]);
    run([
      "cargo",
      "build",
      "--release",
      "--locked",
      "--target",
      target,
      "-p",
      "hiero",
      "-p",
      "hiero-desktop",
      "--features",
      "hiero/console-embed",
      "--bins",
    ]);
    if (target.includes("apple"))
      run([
        "cargo",
        "run",
        "--locked",
        "-p",
        "hiero-desktop",
        "--bin",
        "build-icons",
        "--",
        "--out",
        "target/desktop-icons",
      ]);
    mkdirSync(resolve(values.out!), { recursive: true });
    await packageDesktop({
      target,
      input: resolve(
        `${process.env.CARGO_TARGET_DIR ?? "target"}/${target}/release`,
      ),
      runtime: resolve(process.env.HIERO_RELEASE_RUNTIME_DIR),
      common: resolve(process.env.HIERO_COMMON_MODEL_DIR),
      out: resolve(values.out!),
    });
  }
}
