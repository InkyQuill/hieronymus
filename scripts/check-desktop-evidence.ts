#!/usr/bin/env bun
/** Final-byte qualification gate. Checks attestations, never invents native observations. */
import { lstatSync, readFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { parseArgs } from "node:util";
import { digest } from "./check-rust-release";
import { TARGETS, desktopTarget, readReleaseV2 } from "./desktop-targets";

const COMMON = [
  "native-semantic-mcp",
  "startup",
  "duplicate-start",
  "model-acquisition",
  "semantic-failure",
  "provider-failure-recovery-untested",
  "three-probe-failures",
  "restart",
  "failed-stop",
  "quit-preserves-login",
  "login-toggle",
  "next-login",
  "interactive-menu",
  "missing-tray-host",
  "host-restart",
  "theme",
  "dpi",
  "console-grant",
  "active-upgrade",
  "rollback",
  "uninstall",
  "secret-free-diagnostics",
  "pending-native-operation",
  "private-storage-failure",
];
export function requiredChecks(target: string): string[] {
  desktopTarget(target);
  return [
    ...COMMON,
    ...(target.includes("windows")
      ? ["windows-acl", "scheduler-continuation", "executable-in-use"]
      : target.includes("apple")
        ? ["launchagent-loaded-state", "bundle-launch"]
        : ["native-manager-update", "live-tray-replacement"]),
  ];
}
export interface Identity {
  candidate_commit: string;
  target: string;
  artifact_sha256: string;
  model_sha256: string;
  metadata_sha256: string;
  assets_sha256: string;
  signing: string;
}
export function localFile(root: string, path: string): string {
  if (
    !/^[A-Za-z0-9_./-]+$/.test(path) ||
    path.split("/").some((p) => !p || p === "." || p === "..")
  )
    throw new Error("unsafe evidence path");
  let current = resolve(root);
  if (!lstatSync(current).isDirectory() || lstatSync(current).isSymbolicLink())
    throw new Error("evidence root must be directory");
  const parts = path.split("/");
  for (let i = 0; i < parts.length; i++) {
    current = join(current, parts[i]);
    const st = lstatSync(current);
    if (
      st.isSymbolicLink() ||
      (i === parts.length - 1
        ? !st.isFile() || st.nlink !== 1
        : !st.isDirectory())
    )
      throw new Error("evidence must be regular and unaliased");
  }
  return current;
}
export async function verifyFile(root: string, path: string, hash: string) {
  if (
    !/^[a-f0-9]{64}$/.test(hash) ||
    (await digest(localFile(root, path))) !== hash
  )
    throw new Error(`digest mismatch: ${path}`);
}
export async function validateRecord(
  record: any,
  root: string,
  identity: Identity,
): Promise<void> {
  // The owner permits partial native coverage, provided every gap is explicit.
  // Older records retain their strict fully-qualified semantics.
  const qualification = record?.qualification ?? "qualified";
  if (!["qualified", "partial", "unqualified"].includes(qualification))
    throw new Error("invalid qualification status");
  const limited = qualification !== "qualified";
  const fields = [
    ...Object.keys(identity),
    "os_version",
    "desktop",
    "session",
    "scale",
    "checks",
    ...(record?.qualification !== undefined ? ["qualification"] : []),
    ...(limited ? ["qualification_reason"] : []),
  ].sort();
  if (
    !record ||
    typeof record !== "object" ||
    Object.keys(record).sort().join("\n") !== fields.join("\n")
  )
    throw new Error("qualification fields mismatch");
  if (
    limited &&
    (typeof record.qualification_reason !== "string" ||
      record.qualification_reason.trim().length < 20)
  )
    throw new Error("limited qualification requires a concrete reason");
  for (const [key, value] of Object.entries(identity))
    if (record[key] !== value)
      throw new Error(`qualification identity mismatch: ${key}`);
  if (!/^[a-f0-9]{40}$/.test(record.candidate_commit))
    throw new Error("invalid candidate_commit");
  if (
    typeof record.os_version !== "string" ||
    !record.os_version.trim() ||
    record.os_version.length > 256
  )
    throw new Error("os_version required");
  if (
    !Array.isArray(record.scale) ||
    record.scale.length <
      (qualification === "unqualified" ? 0 : limited ? 1 : 2) ||
    !record.scale.every(
      (s: any) => typeof s === "number" && s >= 1 && s <= 4,
    ) ||
    (!limited &&
      (!record.scale.includes(1) || !record.scale.some((s: number) => s > 1)))
  )
    throw new Error("scale must cover standard and high DPI");
  const platform = record.target.includes("linux")
    ? ["kde", "gnome-appindicator"]
    : record.target.includes("windows")
      ? ["windows"]
      : ["macos"];
  if (
    !platform.includes(record.desktop) ||
    !(
      record.target.includes("linux") ? ["wayland", "x11"] : ["native"]
    ).includes(record.session)
  )
    throw new Error("unsupported desktop/session");
  if (record.signing !== "unsigned-waiver")
    throw new Error(
      "current producer supports explicit unsigned waiver only; transformed signed bytes need a reviewed producer",
    );
  if (!Array.isArray(record.checks)) throw new Error("checks required");
  const seen = new Set<string>();
  for (const check of record.checks) {
    if (
      !check ||
      Object.keys(check).sort().join("\n") !==
        ["name", "result", "evidence_path", "evidence_sha256"].sort().join("\n")
    )
      throw new Error("check fields mismatch");
    if (seen.has(check.name)) throw new Error(`duplicate check: ${check.name}`);
    if (!requiredChecks(record.target).includes(check.name))
      throw new Error(`unknown check: ${check.name}`);
    seen.add(check.name);
  }
  for (const name of requiredChecks(record.target)) {
    const check = record.checks.find((c: any) => c.name === name);
    if (!check) throw new Error(`missing required check: ${name}`);
    if (check.result !== "pass" && !(limited && check.result === "unavailable"))
      throw new Error(`required check must pass: ${name}`);
    await verifyFile(root, check.evidence_path, check.evidence_sha256);
    if (lstatSync(localFile(root, check.evidence_path)).size === 0)
      throw new Error(`empty evidence: ${name}`);
  }
  const passed = record.checks.filter(
    (check: any) => check.result === "pass",
  ).length;
  if (qualification === "unqualified" && passed !== 0)
    throw new Error("unqualified session cannot claim passed native checks");
  if (
    qualification === "partial" &&
    (passed === 0 || passed === record.checks.length)
  )
    throw new Error("partial qualification must record both passes and gaps");
}
export async function validateMatrix(
  release: string,
  evidence: string,
  commit: string,
) {
  if (!/^[a-f0-9]{40}$/.test(commit))
    throw new Error("exact candidate commit required");
  const indexFile = localFile(evidence, "records.json");
  if (lstatSync(indexFile).size > 65536)
    throw new Error("oversized evidence index");
  const index = JSON.parse(readFileSync(indexFile, "utf8"));
  if (!Array.isArray(index))
    throw new Error("records.json must list record paths");
  const records = index.map((path) => {
    const file = localFile(evidence, path);
    if (lstatSync(file).size > 65536)
      throw new Error("oversized evidence record");
    return JSON.parse(readFileSync(file, "utf8"));
  });
  const expectedCount = 7; // KDE and GNOME × Wayland and X11, Windows, both Macs.
  if (records.length !== expectedCount)
    throw new Error(
      "seven-session coverage inventory required, with explicit qualification gaps",
    );
  let common: string | undefined;
  for (const target of TARGETS) {
    if (desktopTarget(target).runtime.origin === "source-build-required")
      throw new Error(
        "Intel runtime promotion missing; four-target release blocked",
      );
    const metadataName = desktopTarget(target).metadata;
    const metadata = readReleaseV2(localFile(release, metadataName), target);
    await verifyFile(
      release,
      metadata.platform.archive,
      metadata.platform.sha256,
    );
    await verifyFile(release, metadata.model.archive, metadata.model.sha256);
    if (common && common !== metadata.model.sha256)
      throw new Error("common model digest differs across targets");
    common = metadata.model.sha256;
    const receipt = JSON.parse(
      readFileSync(localFile(release, `evidence-${target}.json`), "utf8"),
    );
    const assets = `assets-${target}.json`;
    await verifyFile(release, assets, receipt.assets_sha256);
    const identity: Identity = {
      candidate_commit: commit,
      target,
      artifact_sha256: metadata.platform.sha256,
      model_sha256: metadata.model.sha256,
      metadata_sha256: await digest(localFile(release, metadataName)),
      assets_sha256: receipt.assets_sha256,
      signing: "unsigned-waiver",
    };
    if (
      receipt.metadata_sha256 !== identity.metadata_sha256 ||
      receipt.platform_sha256 !== identity.artifact_sha256 ||
      receipt.model_sha256 !== identity.model_sha256 ||
      receipt.signing !==
        "unsigned; no signing identity or notarization supplied"
    )
      throw new Error("producer receipt does not bind final artifacts");
    const sessions = target.includes("linux")
      ? [
          "kde/wayland",
          "kde/x11",
          "gnome-appindicator/wayland",
          "gnome-appindicator/x11",
        ]
      : [target.includes("windows") ? "windows/native" : "macos/native"];
    for (const session of sessions) {
      const matches = records.filter(
        (r) => r.target === target && `${r.desktop}/${r.session}` === session,
      );
      if (matches.length !== 1)
        throw new Error(
          `missing or duplicate matrix record: ${target} ${session}`,
        );
      await validateRecord(matches[0], evidence, identity);
    }
  }
}
if (import.meta.main) {
  const { values } = parseArgs({
    args: Bun.argv.slice(2),
    options: {
      "release-dir": { type: "string" },
      "evidence-dir": { type: "string" },
      commit: { type: "string" },
    },
  });
  if (!values["release-dir"] || !values["evidence-dir"] || !values.commit)
    throw new Error("--release-dir, --evidence-dir and --commit required");
  await validateMatrix(
    values["release-dir"],
    values["evidence-dir"],
    values.commit,
  );
  console.log("Complete native evidence matches final candidate bytes");
}
