#!/usr/bin/env bun
import { resolve, join } from "node:path";
import { desktopTarget } from "./desktop-targets";
const target = desktopTarget(process.env.DESKTOP_TARGET ?? "");
if (
  !["linux/x64", "win32/x64", "darwin/arm64", "darwin/x64"].includes(
    `${process.platform}/${process.arch}`,
  )
)
  throw new Error("unsupported native host architecture");
const native =
  process.platform === "win32"
    ? "x86_64-pc-windows-msvc"
    : process.platform === "darwin"
      ? process.arch === "arm64"
        ? "aarch64-apple-darwin"
        : "x86_64-apple-darwin"
      : "x86_64-unknown-linux-gnu";
if (native !== target.target) throw new Error("native target mismatch");
const cli = resolve(
  `${process.env.CARGO_TARGET_DIR ?? "target"}/${target.target}/release/${target.executable}`,
);
const payload = resolve("target/native-disposable-payload");
const verify = Bun.spawnSync(
  [
    cli,
    "release-verify",
    "--release-dir",
    resolve("target/release-dist"),
    "--output",
    payload,
  ],
  { stdout: "inherit", stderr: "inherit" },
);
if (verify.exitCode !== 0) throw new Error("final pair assembly failed");
// Direct extracted installed layout, no registration/activation on CI hosts.
const child = Bun.spawn(
  [
    "cargo",
    "test",
    "-p",
    "hiero",
    "--locked",
    "--test",
    "desktop_native",
    "final_native_payload_runs_semantic_inference_and_authenticated_mcp",
    "--",
    "--ignored",
    "--nocapture",
  ],
  {
    env: {
      ...process.env,
      HIERO_DESKTOP_INSTALLED_CLI: join(payload, target.executable),
    },
    stdout: Bun.file("target/native-qualification.log"),
    stderr: "inherit",
  },
);
if ((await child.exited) !== 0)
  throw new Error("final native fixture qualification failed");
