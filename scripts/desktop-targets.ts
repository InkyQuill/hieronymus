/** Exact release identities. Pins bind official archives or reviewed native source builds. */
import { openSync, closeSync, constants, fstatSync, readSync } from "node:fs";
import runtimePins from "./onnxruntime-targets.json";
export const TARGETS = [
  "x86_64-unknown-linux-gnu",
  "x86_64-pc-windows-msvc",
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
] as const;
export type DesktopTarget = (typeof TARGETS)[number];
export const MODEL_NAME = "paraphrase-multilingual-MiniLM-L12-v2";
export const MODEL_REVISION = "e8f8c211226b894fcb81acc59f3b34ba3efd5f42";
export const MODEL_PINS = {
  "model.onnx":
    "10f7a088420252b26caf819236ca2c9d2987afd0fc06fec7553b542a5655a05a",
  "tokenizer.json":
    "2c3387be76557bd40970cec13153b3bbf80407865484b209e655e5e4729076b8",
  LICENSE: "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
  "README.md":
    "1e98ea05b0de579fcaad3d625b62ea55647142ed674d5f5ebf1440e4bbbb6f23",
};
export interface OfficialRuntime {
  origin: "official" | "source-reviewed";
  archive: string;
  url: string;
  sha256: string;
  size: number;
  top: string;
  member: string;
  members: Record<string, { sha256: string; size: number }>;
  source?: {
    repository: string;
    revision: string;
    run: string;
    receipt_sha256: string;
  };
}
export interface SourceRuntime {
  origin: "source-build-required";
  version: string;
  repository: string;
  revision: string;
  member: string;
}
export function desktopTarget(value: string) {
  if (!TARGETS.includes(value as DesktopTarget))
    throw new Error(`unsupported desktop target: ${value}`);
  const target = value as DesktopTarget;
  return {
    target,
    architecture: target.startsWith("aarch64") ? "aarch64" : "x86_64",
    extension: target.includes("windows") ? ".zip" : ".tar.gz",
    metadata: `release-${target}.json`,
    executable: target.includes("windows") ? "hiero.exe" : "hiero",
    runtime: runtimePins[target] as OfficialRuntime | SourceRuntime,
  };
}
export function runtimeName(target: DesktopTarget): string {
  return desktopTarget(target).runtime.member.slice(4);
}
export function platformArtifactName(
  version: string,
  target: DesktopTarget,
): string {
  return `hieronymus-${version}-${target}${desktopTarget(target).extension}`;
}
export function modelArtifactName(): string {
  return `hieronymus-model-${MODEL_NAME}-${MODEL_REVISION}.tar.gz`;
}
export function modelMembers(): Record<string, string> {
  return Object.fromEntries(
    Object.entries(MODEL_PINS).map(([name, hash]) => [
      `models/minilm/${name}`,
      hash,
    ]),
  );
}
export interface Artifact {
  archive: string;
  sha256: string;
}
export interface ModelArtifact extends Artifact {
  name: string;
  revision: string;
  members: Record<string, string>;
}
export interface ReleaseV2 {
  format_version: 2;
  version: string;
  target: DesktopTarget;
  channel: "stable" | "dev";
  platform: Artifact;
  model: ModelArtifact;
  signature: null;
}
/** JSON.parse drops duplicate fields. Walk the already syntax-checked document first. */
export function strictJson(text: string): any {
  if (Buffer.byteLength(text) > 65536)
    throw new Error("metadata exceeds byte bound");
  const value = JSON.parse(text);
  let i = 0;
  const space = () => {
    while (/\s/.test(text[i] ?? "x")) i++;
  };
  const string = () => {
    const start = i++;
    while (text[i] !== '"') {
      if (text[i] === "\\") i++;
      i++;
    }
    i++;
    return JSON.parse(text.slice(start, i));
  };
  function walk(depth: number): void {
    if (depth > 64) throw new Error("metadata nesting bound");
    space();
    if (text[i] === "{") {
      i++;
      space();
      const keys = new Set();
      if (text[i] === "}") {
        i++;
        return;
      }
      for (;;) {
        space();
        const key = string();
        if (keys.has(key)) throw new Error("duplicate metadata field");
        keys.add(key);
        space();
        i++;
        walk(depth + 1);
        space();
        if (text[i++] === "}") break;
      }
    } else if (text[i] === "[") {
      i++;
      space();
      if (text[i] === "]") {
        i++;
        return;
      }
      for (;;) {
        walk(depth + 1);
        space();
        if (text[i++] === "]") break;
      }
    } else if (text[i] === '"') string();
    else {
      while (i < text.length && !/[\s,}\]]/.test(text[i])) i++;
    }
  }
  walk(0);
  return value;
}
function keys(value: any, expected: string[]): void {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.keys(value).sort().join("\n") !== [...expected].sort().join("\n")
  )
    throw new Error("metadata fields mismatch");
}
export function parseReleaseV2(text: string, target: DesktopTarget): ReleaseV2 {
  const m = strictJson(text);
  keys(m, [
    "format_version",
    "version",
    "target",
    "channel",
    "platform",
    "model",
    "signature",
  ]);
  if (
    m.format_version !== 2 ||
    m.target !== target ||
    typeof m.version !== "string" ||
    !/^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?$/.test(m.version) ||
    !["stable", "dev"].includes(m.channel) ||
    m.signature !== null
  )
    throw new Error("release identity mismatch");
  desktopTarget(m.target);
  keys(m.platform, ["archive", "sha256"]);
  keys(m.model, ["name", "revision", "archive", "sha256", "members"]);
  if (
    m.platform.archive !== platformArtifactName(m.version, target) ||
    m.model.archive !== modelArtifactName() ||
    m.model.name !== MODEL_NAME ||
    m.model.revision !== MODEL_REVISION
  )
    throw new Error("artifact identity mismatch");
  for (const a of [m.platform, m.model])
    if (typeof a.sha256 !== "string" || !/^[a-f0-9]{64}$/.test(a.sha256))
      throw new Error("artifact digest mismatch");
  const pins = modelMembers();
  keys(m.model.members, Object.keys(pins));
  for (const [name, hash] of Object.entries(pins))
    if (m.model.members[name] !== hash)
      throw new Error("model member mismatch");
  return m;
}

export function readReleaseV2(path: string, target: DesktopTarget): ReleaseV2 {
  const fd = openSync(
    path,
    constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK,
  );
  try {
    const stat = fstatSync(fd);
    if (!stat.isFile() || stat.nlink !== 1 || stat.size > 65536)
      throw new Error("metadata type or size mismatch");
    const bytes = Buffer.alloc(65537);
    let size = 0;
    while (size < bytes.length) {
      const count = readSync(fd, bytes, size, bytes.length - size, null);
      if (!count) break;
      size += count;
    }
    return parseReleaseV2(bytes.subarray(0, size).toString("utf8"), target);
  } finally {
    closeSync(fd);
  }
}
