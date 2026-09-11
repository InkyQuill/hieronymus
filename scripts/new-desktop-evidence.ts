#!/usr/bin/env bun
/** Start a native observation record honestly; every check remains unavailable. */
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, basename, join } from "node:path";
import { release, platform, arch } from "node:os";
import { desktopTarget, readReleaseV2 } from "./desktop-targets";
import { digest } from "./check-rust-release";
import { requiredChecks, type Identity } from "./check-desktop-evidence";
export function observationTemplate(
  identity: Identity,
  desktop: string,
  session: string,
  stem: string,
) {
  if (
    !/^[a-f0-9]{40}$/.test(identity.candidate_commit) ||
    !/^[a-z0-9-]+$/.test(stem)
  )
    throw new Error("exact commit and simple record filename required");
  return {
    ...identity,
    os_version: `${platform()} ${release()} ${arch()}`,
    desktop,
    session,
    scale: [1],
    checks: requiredChecks(identity.target).map((name) => ({
      name,
      result: "unavailable",
      evidence_path: `captures/${stem}/${name}.txt`,
      evidence_sha256: null,
    })),
  };
}
if (import.meta.main) {
  const [directory, commit, desktop, session, out] = Bun.argv.slice(2);
  if (!directory || !commit || !desktop || !session || !out)
    throw new Error(
      "usage: new-desktop-evidence.ts RELEASE_DIR COMMIT DESKTOP SESSION OUTPUT.json",
    );
  if (
    !["linux/x64", "win32/x64", "darwin/arm64", "darwin/x64"].includes(
      `${platform()}/${arch()}`,
    )
  )
    throw new Error("unsupported native host architecture");
  const target = desktopTarget(
    platform() === "win32"
      ? "x86_64-pc-windows-msvc"
      : platform() === "darwin"
        ? arch() === "arm64"
          ? "aarch64-apple-darwin"
          : "x86_64-apple-darwin"
        : "x86_64-unknown-linux-gnu",
  );
  const metadata = readReleaseV2(
    join(directory, target.metadata),
    target.target,
  );
  const receipt = JSON.parse(
    readFileSync(join(directory, `evidence-${target.target}.json`), "utf8"),
  );
  const record = observationTemplate(
    {
      candidate_commit: commit,
      target: target.target,
      artifact_sha256: metadata.platform.sha256,
      model_sha256: metadata.model.sha256,
      metadata_sha256: await digest(join(directory, target.metadata)),
      assets_sha256: receipt.assets_sha256,
      signing: "unsigned-waiver",
    },
    desktop,
    session,
    basename(out, ".json"),
  );
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, JSON.stringify(record, null, 2) + "\n", { flag: "wx" });
  console.log(
    "Unqualified template created. Record actual observations, scale values and capture hashes; no pass was inferred.",
  );
}
