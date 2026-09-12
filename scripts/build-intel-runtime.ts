#!/usr/bin/env bun
/** Native-only candidate producer. Output is evidence for pin review, never automatic authority. */
import {
  existsSync,
  mkdirSync,
  copyFileSync,
  writeFileSync,
  readFileSync,
  lstatSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { digest } from "./check-rust-release";
export const INTEL_SOURCE = "da9b5e364c465de65c49d91e696cd6485270757f";
export function requireIntelHost(
  platform = process.platform,
  architecture = process.arch,
): void {
  if (platform !== "darwin" || architecture !== "x64")
    throw new Error(
      "ONNX Intel source build requires a native macOS x86_64 host; no cross-build acceptance",
    );
}
function run(command: string[], cwd: string): string {
  const result = Bun.spawnSync(command, {
    cwd,
    stdout: "pipe",
    stderr: "pipe",
  });
  if (result.exitCode !== 0)
    throw new Error(
      `command failed: ${command[0]} (${result.exitCode})\n${result.stderr}`,
    );
  return result.stdout.toString().trim();
}
export async function buildIntelRuntime(
  source: string,
  output: string,
): Promise<void> {
  requireIntelHost();
  source = resolve(source);
  output = resolve(output);
  if (existsSync(output)) throw new Error("source-build output must be absent");
  if (
    run(["git", "rev-parse", "HEAD"], source) !== INTEL_SOURCE ||
    run(
      ["git", "status", "--porcelain", "--untracked-files=normal"],
      source,
    ) !== ""
  )
    throw new Error("source build needs a clean exact ONNX 1.28.0 checkout");
  if (readFileSync(join(source, "VERSION_NUMBER"), "utf8").trim() !== "1.28.0")
    throw new Error("source version mismatch");
  mkdirSync(output, { recursive: true, mode: 0o700 });
  const command = [
    join(source, "build.sh"),
    "--config",
    "Release",
    "--build_shared_lib",
    "--parallel",
    "2",
    "--update",
    "--build",
    "--test",
    "--build_dir",
    join(output, "build"),
    "--cmake_extra_defines",
    "CMAKE_OSX_ARCHITECTURES=x86_64",
    "CMAKE_OSX_DEPLOYMENT_TARGET=14.0",
  ];
  const toolchain = {
    os: run(["sw_vers"], source),
    clang: run(["xcrun", "clang", "--version"], source),
    xcode: run(["xcodebuild", "-version"], source),
    cmake: run(["cmake", "--version"], source),
    python: run(["python3", "--version"], source),
    architecture: run(["uname", "-m"], source),
  };
  writeFileSync(
    join(output, "build-plan.json"),
    JSON.stringify(
      { source_revision: INTEL_SOURCE, command, toolchain },
      null,
      2,
    ),
    { flag: "wx" },
  );
  const log = Bun.file(join(output, "build.log"));
  const child = Bun.spawn(command, { cwd: source, stdout: log, stderr: log });
  if ((await child.exited) !== 0)
    throw new Error("native ONNX build/test failed; inspect build.log");
  const library = join(output, "build/Release/libonnxruntime.1.28.0.dylib");
  if (!lstatSync(library).isFile())
    throw new Error("missing regular exact-version dylib output");
  const architecture = run(["lipo", "-archs", library], source);
  if (architecture !== "x86_64")
    throw new Error("source-build artifact is not exactly x86_64");
  const loadCommands = run(["otool", "-l", library], source),
    dependencies = run(["otool", "-L", library], source);
  writeFileSync(join(output, "macho-load-commands.txt"), loadCommands, {
    flag: "wx",
  });
  writeFileSync(join(output, "macho-dependencies.txt"), dependencies, {
    flag: "wx",
  });
  const top = "onnxruntime-osx-x86_64-1.28.0-source",
    tree = join(output, top);
  mkdirSync(join(tree, "lib"), { recursive: true, mode: 0o700 });
  copyFileSync(library, join(tree, "lib/libonnxruntime.dylib"));
  const names = [
    "lib/libonnxruntime.dylib",
    "LICENSE",
    "ThirdPartyNotices.txt",
    "VERSION_NUMBER",
  ];
  for (const name of names.slice(1))
    copyFileSync(join(source, name), join(tree, name));
  const members: Record<string, { sha256: string; size: number }> = {};
  for (const name of names)
    members[name] = {
      sha256: await digest(join(tree, name)),
      size: lstatSync(join(tree, name)).size,
    };
  const archive = `${top}.tgz`;
  run(["tar", "-czf", join(output, archive), "-C", output, top], source);
  const receipt = {
    format_version: 1,
    status: "candidate-requires-review",
    target: "x86_64-apple-darwin",
    runtime_version: "1.28.0",
    source_repository: "https://github.com/microsoft/onnxruntime.git",
    source_revision: INTEL_SOURCE,
    submodules: run(["git", "submodule", "status", "--recursive"], source),
    command,
    toolchain,
    deployment_target_requested: "14.0",
    architecture,
    archive,
    sha256: await digest(join(output, archive)),
    members,
    build_log_sha256: await digest(join(output, "build.log")),
    load_commands_sha256: await digest(join(output, "macho-load-commands.txt")),
    dependencies_sha256: await digest(join(output, "macho-dependencies.txt")),
    native_model_inference: "pending",
    interactive_desktop: "pending",
  };
  writeFileSync(
    join(output, "source-build-receipt.json"),
    JSON.stringify(receipt, null, 2),
    { flag: "wx" },
  );
}
if (import.meta.main) {
  try {
    const { values } = parseArgs({
      args: Bun.argv.slice(2),
      options: { source: { type: "string" }, out: { type: "string" } },
    });
    if (!values.source || !values.out)
      throw new Error(
        "provide --source <clean pinned checkout> --out <absent evidence directory>",
      );
    await buildIntelRuntime(values.source, values.out);
  } catch (error) {
    console.error(
      error instanceof Error ? error.message : "Intel source build refused",
    );
    process.exitCode = 1;
  }
}
