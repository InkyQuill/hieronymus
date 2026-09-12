import { expect, test } from "bun:test";
import {
  desktopTarget,
  runtimeName,
  modelArtifactName,
  modelMembers,
  parseReleaseV2,
  platformArtifactName,
  TARGETS,
} from "./desktop-targets";
const sha = "a".repeat(64);
function manifest(target = TARGETS[0]) {
  return {
    format_version: 2,
    version: "0.8.0",
    target,
    channel: "stable",
    platform: { archive: platformArtifactName("0.8.0", target), sha256: sha },
    model: {
      name: "paraphrase-multilingual-MiniLM-L12-v2",
      revision: "e8f8c211226b894fcb81acc59f3b34ba3efd5f42",
      archive: modelArtifactName(),
      sha256: sha,
      members: modelMembers(),
    },
    signature: null,
  };
}
test("native runtime names and exact target selection", () => {
  expect(runtimeName("x86_64-pc-windows-msvc")).toBe("onnxruntime.dll");
  expect(runtimeName("aarch64-apple-darwin")).toBe("libonnxruntime.dylib");
  expect(() => desktopTarget("linux-x64")).toThrow();
  expect(desktopTarget("x86_64-apple-darwin").runtime.origin).toBe(
    "source-reviewed",
  );
});
test("all targets bind one common pinned model artifact", () => {
  for (const target of TARGETS)
    expect(
      parseReleaseV2(JSON.stringify(manifest(target)), target).model,
    ).toEqual(manifest().model);
});
for (const corrupt of [
  (m: any) => (m.target = "unknown"),
  (m: any) => (m.platform.archive += ".zip"),
  (m: any) => (m.runtime = "libonnxruntime.so"),
  (m: any) => (m.model.revision = "wrong"),
  (m: any) => delete m.model.members["models/minilm/LICENSE"],
  (m: any) => (m.model.members["../escape"] = sha),
  (m: any) => (m.model.sha256 = "bad"),
  (m: any) => (m.signature = "unverified"),
  (m: any) => (m.format_version = 1),
])
  test("reject malformed split metadata", () => {
    const m = manifest();
    corrupt(m);
    expect(() => parseReleaseV2(JSON.stringify(m), TARGETS[0])).toThrow();
  });
test("duplicate and oversized metadata fields refused", () => {
  const text = JSON.stringify(manifest());
  expect(() =>
    parseReleaseV2(
      text.replace(
        '"format_version":2',
        '"format_version":2,"format_version":2',
      ),
      TARGETS[0],
    ),
  ).toThrow();
  expect(() =>
    parseReleaseV2(
      text.replace('"members":{', '"members":{"models/minilm/LICENSE":"bad",'),
      TARGETS[0],
    ),
  ).toThrow();
  expect(() => parseReleaseV2(text + " ".repeat(65536), TARGETS[0])).toThrow();
});

import { zipMembers, crc32 } from "./runtime-zip";
import { deflateRawSync } from "node:zlib";
function zip(entries: [string, string][]): Buffer {
  const locals: Buffer[] = [],
    centrals: Buffer[] = [];
  let offset = 0;
  for (const [name, value] of entries) {
    const raw = Buffer.from(name),
      data = Buffer.from(value),
      compressed = deflateRawSync(data),
      local = Buffer.alloc(30),
      central = Buffer.alloc(46);
    local.writeUInt32LE(0x04034b50);
    local.writeUInt16LE(8, 8);
    local.writeUInt32LE(crc32(data), 14);
    local.writeUInt32LE(compressed.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(raw.length, 26);
    central.writeUInt32LE(0x02014b50);
    central.writeUInt16LE(8, 10);
    central.writeUInt32LE(crc32(data), 16);
    central.writeUInt32LE(compressed.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(raw.length, 28);
    central.writeUInt32LE(offset, 42);
    locals.push(local, raw, compressed);
    centrals.push(central, raw);
    offset += local.length + raw.length + compressed.length;
  }
  const cd = Buffer.concat(centrals),
    end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(cd.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, cd, end]);
}
test("real ZIP deflate members and local/central binding", () => {
  const bytes = zip([["top/lib/onnxruntime.dll", "runtime"]]);
  expect(
    zipMembers(bytes, 100).get("top/lib/onnxruntime.dll")?.toString(),
  ).toBe("runtime");
  const bad = Buffer.from(bytes);
  bad.writeUInt32LE(100, 22);
  expect(() => zipMembers(bad, 100)).toThrow();
});
for (const entries of [
  [
    ["top/a", "1"],
    ["top/a", "2"],
  ],
  [["top/../escape", "1"]],
  [["/absolute", "1"]],
  [["top/a:stream", "1"]],
  [
    ["top/a", "1"],
    ["top/a/b", "2"],
  ],
] as [string, string][][])
  test("ZIP rejects duplicates, traversal, streams and file parents", () =>
    expect(() => zipMembers(zip(entries), 100)).toThrow());
test("ZIP refuses declared and actual decompression overflows", () => {
  const bytes = zip([["top/a", "x".repeat(10000)]]);
  expect(() => zipMembers(bytes, 100)).toThrow();
  const end = bytes.length - 22,
    cd = bytes.readUInt32LE(end + 16);
  bytes.writeUInt32LE(1, 22);
  bytes.writeUInt32LE(1, cd + 24);
  expect(() => zipMembers(bytes, 100)).toThrow();
});

import { acquireCommonModel } from "./shared-model";
import { mkdtempSync, realpathSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
test("common acquisition cache rehashes every reuse without requests", async () => {
  const dir = mkdtempSync(
    join(realpathSync(tmpdir()), "hiero-common-cache-"),
  );
  try {
    const data = Buffer.from("transport fixture"),
      hash = createHash("sha256").update(data).digest("hex"),
      model = { ...manifest().model, sha256: hash };
    let requests = 0;
    const request = (async () => {
      requests++;
      return new Response(data);
    }) as typeof fetch;
    const path = await acquireCommonModel(
      model,
      dir,
      "https://github.com/model",
      new Set(["github.com"]),
      AbortSignal.timeout(1000),
      request,
    );
    await acquireCommonModel(
      model,
      dir,
      "https://github.com/model",
      new Set(["github.com"]),
      AbortSignal.timeout(1000),
      request,
    );
    expect(requests).toBe(1);
    writeFileSync(path, "corrupt");
    await expect(
      acquireCommonModel(
        model,
        dir,
        "https://github.com/model",
        new Set(["github.com"]),
        AbortSignal.timeout(1000),
        request,
      ),
    ).rejects.toThrow();
    expect(requests).toBe(1);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
import { requireIntelHost } from "./build-intel-runtime";
test("Intel source-build route refuses cross-host acceptance", () => {
  expect(() => requireIntelHost("linux", "x64")).toThrow();
  expect(() => requireIntelHost("darwin", "arm64")).toThrow();
  expect(() => requireIntelHost("darwin", "x64")).not.toThrow();
});
import { checkMetadata } from "./check-rust-release";
test("source checker verifies both artifacts from the exact target manifest", async () => {
  const dir = mkdtempSync(join(realpathSync(tmpdir()), "hiero-v2-check-"));
  try {
    const m = manifest();
    const data = Buffer.from("transport fixture");
    m.platform.sha256 = createHash("sha256").update(data).digest("hex");
    m.model.sha256 = m.platform.sha256;
    writeFileSync(join(dir, m.platform.archive), data);
    writeFileSync(join(dir, m.model.archive), data);
    writeFileSync(
      join(dir, desktopTarget(m.target).metadata),
      JSON.stringify(m),
    );
    expect(await checkMetadata(dir, "0.8.0", "stable")).toBe(
      m.platform.archive,
    );
    writeFileSync(join(dir, m.model.archive), "changed");
    await expect(checkMetadata(dir, "0.8.0", "stable")).rejects.toThrow();
    writeFileSync(join(dir, m.model.archive), data);
    writeFileSync(join(dir, m.platform.archive), "changed");
    await expect(checkMetadata(dir, "0.8.0", "stable")).rejects.toThrow();
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
import { directoryPrefixes } from "./release-assets";
import { win32, posix } from "node:path";
test("trusted directory traversal preserves Windows drive and UNC roots", () => {
  expect(directoryPrefixes("C:\\work\\cache", win32)).toEqual([
    "C:\\",
    "C:\\work",
    "C:\\work\\cache",
  ]);
  expect(directoryPrefixes("\\\\server\\share\\work\\cache", win32)).toEqual([
    "\\\\server\\share\\",
    "\\\\server\\share\\work",
    "\\\\server\\share\\work\\cache",
  ]);
  expect(directoryPrefixes("/work/cache", posix)).toEqual([
    "/",
    "/work",
    "/work/cache",
  ]);
});
