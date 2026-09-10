#!/usr/bin/env bun
/** Bind release metadata to workspace sources, an exact tag, and archive bytes. */
import { createHash } from "node:crypto";
import { createReadStream, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { parseArgs } from "node:util";

export const TARGET = "x86_64-unknown-linux-gnu";
export async function digest(path: string): Promise<string> {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}
function git(root: string, ref: string): string {
  const result = Bun.spawnSync(
    ["git", "-C", root, "rev-parse", "--verify", ref],
    { stderr: "pipe" },
  );
  if (result.exitCode !== 0) throw new Error("release tag does not resolve");
  return result.stdout.toString().trim();
}
export function checkSource(
  root: string,
  ref?: string,
  allowUntagged = false,
): string {
  const version = (
    Bun.TOML.parse(readFileSync(resolve(root, "Cargo.toml"), "utf8")) as any
  ).workspace.package.version;
  if (
    typeof version !== "string" ||
    !/^0\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)
  )
    throw new Error(
      "Rust release requires an explicit alpha 0.x.y version (ADR 0006)",
    );
  const packages = (
    Bun.TOML.parse(readFileSync(resolve(root, "Cargo.lock"), "utf8")) as any
  ).package;
  for (const name of ["hiero", "hieronymus"]) {
    const versions = packages.filter((item: any) => item.name === name);
    if (versions.length !== 1 || versions[0].version !== version)
      throw new Error(`${name} Cargo.lock version mismatch`);
    const crate = Bun.TOML.parse(
      readFileSync(resolve(root, "crates", name, "Cargo.toml"), "utf8"),
    ) as any;
    if (JSON.stringify(crate.package.version) !== '{"workspace":true}')
      throw new Error(`${name} must inherit the workspace version`);
  }
  if (ref === undefined && allowUntagged) return version;
  if (ref !== `refs/tags/v${version}`)
    throw new Error("release ref must be the exact workspace-version tag");
  if (git(root, "HEAD") !== git(root, `${ref}^{commit}`))
    throw new Error("release tag does not point to checked-out HEAD");
  return version;
}
export async function checkMetadata(
  directory: string,
  version: string,
  channel: string,
): Promise<string> {
  if (!["stable", "dev"].includes(channel))
    throw new Error("unsupported release channel");
  const metadata = JSON.parse(
    readFileSync(resolve(directory, "release.json"), "utf8"),
  );
  const archive = `hieronymus-${version}-${TARGET}.tar.gz`;
  const expected = {
    version,
    target: TARGET,
    channel,
    archive,
    signature: null,
  };
  if (Object.entries(expected).some(([key, value]) => metadata[key] !== value))
    throw new Error(
      "release metadata does not match source/target/channel/waiver",
    );
  if (metadata.sha256 !== (await digest(resolve(directory, archive))))
    throw new Error("release archive checksum mismatch");
  return archive;
}
if (import.meta.main) {
  try {
    const { values } = parseArgs({
      args: Bun.argv.slice(2),
      options: {
        root: { type: "string", default: resolve(import.meta.dir, "..") },
        ref: { type: "string" },
        "allow-untagged": { type: "boolean" },
        "release-dir": { type: "string" },
        channel: { type: "string", default: "stable" },
      },
    });
    const version = checkSource(
      values.root!,
      values.ref,
      values["allow-untagged"],
    );
    const archive = values["release-dir"]
      ? await checkMetadata(values["release-dir"], version, values.channel!)
      : null;
    console.log(
      JSON.stringify({
        version,
        archive,
        tag_verified: values.ref !== undefined,
      }),
    );
  } catch (error) {
    console.error(
      `Rust release refused: ${error instanceof Error ? error.message : "invalid input"}`,
    );
    process.exitCode = 1;
  }
}
