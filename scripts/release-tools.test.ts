import { afterEach, describe, expect, test } from "bun:test";
import {
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  readFileSync,
  rmSync,
  symlinkSync,
  existsSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { checkSource, checkMetadata, digest } from "./check-rust-release";
import {
  safeUrl,
  MODEL_HOSTS,
  RUNTIME_HOSTS,
  download,
  verify,
  extractTar,
  trustedDirectory,
  unpackRuntime,
} from "./release-assets";
import { gzipSync } from "node:zlib";
const roots: string[] = [];
function temp() {
  const root = mkdtempSync(join(tmpdir(), "hiero-release-test-"));
  roots.push(root);
  return root;
}
afterEach(() => {
  for (const root of roots.splice(0))
    rmSync(root, { recursive: true, force: true });
});
function git(root: string, ...args: string[]) {
  const result = Bun.spawnSync(["git", "-C", root, ...args], {
    stderr: "pipe",
  });
  if (result.exitCode) throw new Error(result.stderr.toString());
}
function source() {
  const root = temp();
  writeFileSync(
    join(root, "Cargo.toml"),
    '[workspace.package]\nversion="0.8.0"\n',
  );
  writeFileSync(
    join(root, "Cargo.lock"),
    '[[package]]\nname="hiero"\nversion="0.8.0"\n[[package]]\nname="hieronymus"\nversion="0.8.0"\n',
  );
  for (const name of ["hiero", "hieronymus"]) {
    mkdirSync(join(root, "crates", name), { recursive: true });
    writeFileSync(
      join(root, "crates", name, "Cargo.toml"),
      "[package]\nversion.workspace=true\n",
    );
  }
  git(root, "init", "-q");
  git(root, "add", ".");
  git(
    root,
    "-c",
    "user.name=Fixture",
    "-c",
    "user.email=fixture@example.invalid",
    "commit",
    "-qm",
    "fixture",
  );
  git(root, "tag", "v0.8.0");
  return root;
}
test("exact tag and explicit untagged candidate", () => {
  const root = source();
  expect(checkSource(root, "refs/tags/v0.8.0")).toBe("0.8.0");
  expect(checkSource(root, undefined, true)).toBe("0.8.0");
  expect(() => checkSource(root)).toThrow();
});
for (const ref of [
  "refs/heads/main",
  "refs/tags/v0.9.0",
  "refs/tags/v0.8.0;echo bad",
])
  test(`reject wrong ref ${ref}`, () =>
    expect(() => checkSource(source(), ref)).toThrow());
test("tag must point to checkout", () => {
  const root = source();
  writeFileSync(join(root, "new"), "new");
  git(root, "add", ".");
  git(
    root,
    "-c",
    "user.name=Fixture",
    "-c",
    "user.email=fixture@example.invalid",
    "commit",
    "-qm",
    "new",
  );
  expect(() => checkSource(root, "refs/tags/v0.8.0")).toThrow(
    "checked-out HEAD",
  );
});
for (const broken of ["version", "lock", "inheritance"])
  test(`version binding ${broken}`, () => {
    const root = source();
    const path = join(
      root,
      broken === "version"
        ? "Cargo.toml"
        : broken === "lock"
          ? "Cargo.lock"
          : "crates/hiero/Cargo.toml",
    );
    writeFileSync(
      path,
      broken === "inheritance"
        ? '[package]\nversion="0.8.0"'
        : readFileSync(path, "utf8").replaceAll("0.8.0", "1.0.0"),
    );
    expect(() => checkSource(root, undefined, true)).toThrow();
  });
async function release() {
  const root = temp(),
    archive = "hieronymus-0.8.0-x86_64-unknown-linux-gnu.tar.gz";
  writeFileSync(join(root, archive), "archive fixture");
  const metadata: Record<string, unknown> = {
    version: "0.8.0",
    target: "x86_64-unknown-linux-gnu",
    channel: "stable",
    archive,
    sha256: await digest(join(root, archive)),
    signature: null,
  };
  writeFileSync(join(root, "release.json"), JSON.stringify(metadata));
  return { root, metadata };
}
test("metadata binds exact archive", async () => {
  const { root, metadata } = await release();
  expect(await checkMetadata(root, "0.8.0", "stable")).toBe(metadata.archive);
});
for (const [key, value] of [
  ["version", "0.7.0"],
  ["target", "other"],
  ["channel", "dev"],
  ["archive", "../escape.tar.gz"],
  ["sha256", "0".repeat(64)],
  ["signature", "unexpected"],
])
  test(`corrupt metadata ${key}`, async () => {
    const { root, metadata } = await release();
    metadata[key] = value;
    writeFileSync(join(root, "release.json"), JSON.stringify(metadata));
    await expect(checkMetadata(root, "0.8.0", "stable")).rejects.toThrow();
  });
for (const value of [
  "http://huggingface.co/file",
  "https://huggingface.co:443/file",
  "https://user@huggingface.co/file",
  "https://evil.example/file",
  "https://huggingface.co/file#fragment",
])
  test(`refuse unsafe URL ${value}`, () =>
    expect(() => safeUrl(value, MODEL_HOSTS)).toThrow());
test("approved redirect hosts allowed", () => {
  for (const host of [...MODEL_HOSTS, ...RUNTIME_HOSTS])
    expect(safeUrl(`https://${host}/file`, new Set([host])).hostname).toBe(
      host,
    );
});
test("redirect checked before contacting untrusted host", async () => {
  let count = 0;
  const request = (async () => {
    count++;
    return new Response(null, {
      status: 302,
      headers: { location: "https://evil.example/secret" },
    });
  }) as typeof fetch;
  await expect(
    download(
      "https://huggingface.co/file",
      join(temp(), "part"),
      MODEL_HOSTS,
      10,
      AbortSignal.timeout(1000),
      request,
    ),
  ).rejects.toThrow();
  expect(count).toBe(1);
});
test("redirect count bounded", async () => {
  let count = 0;
  const request = (async () => {
    count++;
    return new Response(null, { status: 302, headers: { location: "/next" } });
  }) as typeof fetch;
  await expect(
    download(
      "https://huggingface.co/file",
      join(temp(), "part"),
      MODEL_HOSTS,
      10,
      AbortSignal.timeout(1000),
      request,
    ),
  ).rejects.toThrow();
  expect(count).toBe(6);
});
for (const headers of [
  {},
  { "content-length": "11" },
  { "content-length": "4" },
])
  test(`bounded or truncated transfer ${JSON.stringify(headers)}`, async () => {
    const request = (async () =>
      new Response("12345678901", { headers })) as typeof fetch;
    await expect(
      download(
        "https://huggingface.co/file",
        join(temp(), "part"),
        MODEL_HOSTS,
        10,
        AbortSignal.timeout(1000),
        request,
      ),
    ).rejects.toThrow();
  });
test("successful bounded transfer and checksum verification", async () => {
  const path = join(temp(), "asset");
  const request = (async () =>
    new Response("data", {
      headers: { "content-length": "4" },
    })) as typeof fetch;
  await download(
    "https://huggingface.co/file",
    path,
    MODEL_HOSTS,
    4,
    AbortSignal.timeout(1000),
    request,
  );
  const hash = createHash("sha256").update("data").digest("hex");
  await verify(path, hash, 4);
  writeFileSync(path, "oops");
  await expect(verify(path, hash)).rejects.toThrow();
});
test("symlink cache and directory refused", async () => {
  const root = temp();
  writeFileSync(join(root, "real"), "data");
  symlinkSync(join(root, "real"), join(root, "link"));
  await expect(
    verify(join(root, "link"), await digest(join(root, "real"))),
  ).rejects.toThrow();
  symlinkSync(root, join(root, "dir"));
  expect(() => trustedDirectory(join(root, "dir"))).toThrow();
});
const top = "onnxruntime-linux-x64-1.28.0";
function tar(
  entries: { name: string; type?: string; link?: string; data?: string }[],
) {
  const blocks: Buffer[] = [];
  for (const entry of entries) {
    const data = Buffer.from(entry.data ?? ""),
      header = Buffer.alloc(512);
    header.write(entry.name, 0, 100);
    header.write("0000600\0", 100);
    header.write("0000000\0", 108);
    header.write("0000000\0", 116);
    header.write(data.length.toString(8).padStart(11, "0") + "\0", 124);
    header.fill(32, 148, 156);
    header.write(entry.type ?? "0", 156);
    header.write(entry.link ?? "", 157, 100);
    header.write(
      [...header]
        .reduce((a, b) => a + b, 0)
        .toString(8)
        .padStart(6, "0") + "\0 ",
      148,
    );
    blocks.push(header, data, Buffer.alloc((512 - (data.length % 512)) % 512));
  }
  return Buffer.concat([...blocks, Buffer.alloc(1024)]);
}
test("extract regular files and materialize internal runtime link", () => {
  const root = temp();
  extractTar(
    tar([
      { name: `${top}/lib/runtime`, data: "runtime" },
      { name: `${top}/lib/libonnxruntime.so`, type: "2", link: "runtime" },
    ]),
    root,
  );
  expect(readFileSync(join(root, top, "lib/libonnxruntime.so"), "utf8")).toBe(
    "runtime",
  );
});
for (const name of [
  "../escape",
  "/absolute",
  `${top}/../escape`,
  `${top}//file`,
  `${top}/./file`,
])
  test(`tar traversal ${name}`, () =>
    expect(() => extractTar(tar([{ name, data: "bad" }]), temp())).toThrow());
for (const type of ["1", "3", "4", "6", "x", "L"])
  test(`tar unsupported type ${type}`, () =>
    expect(() =>
      extractTar(tar([{ name: `${top}/bad`, type }]), temp()),
    ).toThrow());
test("duplicate paths, symlink parents and external links refused", () => {
  for (const entries of [
    [{ name: `${top}/x` }, { name: `${top}/x` }],
    [
      { name: `${top}/link`, type: "2", link: "file" },
      { name: `${top}/link/child` },
      { name: `${top}/file` },
    ],
    [{ name: `${top}/link`, type: "2", link: "../../outside" }],
  ])
    expect(() => extractTar(tar(entries), temp())).toThrow();
});
test("corrupt header and truncated archive refused", () => {
  const bytes = tar([{ name: `${top}/file`, data: "abc" }]);
  expect(() => extractTar(bytes.subarray(0, 1024), temp())).toThrow();
  bytes[0] ^= 1;
  expect(() => extractTar(bytes, temp())).toThrow();
});
test("gzip extraction is exercised without native assets", async () => {
  const root = temp(),
    out = temp(),
    path = join(root, "runtime.tgz");
  writeFileSync(path, gzipSync(tar([{ name: `${top}/file`, data: "abc" }])));
  await unpackRuntime(path, out, AbortSignal.timeout(1000));
  expect(readFileSync(join(out, top, "file"), "utf8")).toBe("abc");
});

test("native two-step runtime links materialize without symlinks", () => {
  const root = temp();
  extractTar(
    tar([
      { name: `${top}/lib/runtime.1.28`, data: "runtime" },
      { name: `${top}/lib/runtime.1`, type: "2", link: "runtime.1.28" },
      { name: `${top}/lib/runtime`, type: "2", link: "runtime.1" },
    ]),
    root,
  );
  expect(readFileSync(join(root, top, "lib/runtime"), "utf8")).toBe("runtime");
});
test("link cycles and nonempty extraction directories refused", () => {
  expect(() =>
    extractTar(
      tar([
        { name: `${top}/a`, type: "2", link: "b" },
        { name: `${top}/b`, type: "2", link: "a" },
      ]),
      temp(),
    ),
  ).toThrow("cyclic");
  const root = temp();
  writeFileSync(join(root, "existing"), "keep");
  expect(() => extractTar(tar([{ name: `${top}/file` }]), root)).toThrow(
    "must be empty",
  );
  expect(readFileSync(join(root, "existing"), "utf8")).toBe("keep");
});
test("gzip decompression bound enforced before extraction", async () => {
  const root = temp(),
    out = temp(),
    path = join(root, "runtime.tgz");
  writeFileSync(
    path,
    gzipSync(tar([{ name: `${top}/file`, data: "x".repeat(10000) }])),
  );
  await expect(
    unpackRuntime(path, out, AbortSignal.timeout(1000), 1024),
  ).rejects.toThrow("bound");
  expect(existsSync(join(out, top))).toBe(false);
});
test("link materialization cannot amplify beyond extraction bound", () => {
  const entries = [
    { name: `${top}/file`, data: "x".repeat(4000) },
    ...Array.from({ length: 10 }, (_, i) => ({
      name: `${top}/link${i}`,
      type: "2",
      link: "file",
    })),
  ];
  const bytes = tar(entries);
  expect(() => extractTar(bytes, temp(), top, bytes.length + 1)).toThrow(
    "materialized",
  );
});
import { acquireFile } from "./release-assets";
import { readdirSync } from "node:fs";
for (const failure of ["transfer", "wrong-hash", "oversized"])
  test(`failed acquisition is not promoted: ${failure}`, async () => {
    const root = temp(),
      path = join(root, "asset");
    const request = (async () => {
      if (failure === "transfer") throw new Error("sensitive-url");
      return new Response(failure === "oversized" ? "x".repeat(11) : "wrong");
    }) as typeof fetch;
    await expect(
      acquireFile(
        path,
        "0".repeat(64),
        "https://huggingface.co/file",
        MODEL_HOSTS,
        10,
        AbortSignal.timeout(1000),
        request,
      ),
    ).rejects.toThrow();
    expect(existsSync(path)).toBe(false);
    expect(readdirSync(root)).toEqual([]);
  });
test("verified cache reuse performs no request; corrupt cache fails closed", async () => {
  const root = temp(),
    path = join(root, "asset");
  const hash = createHash("sha256").update("data").digest("hex");
  let requests = 0;
  const request = (async () => {
    requests++;
    return new Response("data");
  }) as typeof fetch;
  await acquireFile(
    path,
    hash,
    "https://huggingface.co/file",
    MODEL_HOSTS,
    10,
    AbortSignal.timeout(1000),
    request,
  );
  await acquireFile(
    path,
    hash,
    "https://huggingface.co/file",
    MODEL_HOSTS,
    10,
    AbortSignal.timeout(1000),
    request,
  );
  expect(requests).toBe(1);
  writeFileSync(path, "corrupt");
  await expect(
    acquireFile(
      path,
      hash,
      "https://huggingface.co/file",
      MODEL_HOSTS,
      10,
      AbortSignal.timeout(1000),
      request,
    ),
  ).rejects.toThrow();
  expect(requests).toBe(1);
});
test("dangling cache symlink cannot be replaced", async () => {
  const root = temp(),
    path = join(root, "asset");
  symlinkSync(join(root, "missing"), path);
  let requests = 0;
  const request = (async () => {
    requests++;
    return new Response("data");
  }) as typeof fetch;
  await expect(
    acquireFile(
      path,
      "0".repeat(64),
      "https://huggingface.co/file",
      MODEL_HOSTS,
      10,
      AbortSignal.timeout(1000),
      request,
    ),
  ).rejects.toThrow();
  expect(requests).toBe(0);
  expect(readdirSync(root)).toEqual(["asset"]);
});
