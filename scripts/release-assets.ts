/** Pinned, bounded release acquisition. No package dependencies or shell extraction. */
import {
  constants,
  createReadStream,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  closeSync,
  readFileSync,
  writeFileSync,
  linkSync,
  unlinkSync,
  rmSync,
  realpathSync,
  readdirSync,
  fstatSync,
} from "node:fs";
import { dirname, join, resolve, posix } from "node:path";
import * as nativePath from "node:path";
import {
  desktopTarget,
  MODEL_PINS,
  type DesktopTarget,
} from "./desktop-targets";
import { zipMembers } from "./runtime-zip";
export { MODEL_PINS } from "./desktop-targets";
import { createGunzip } from "node:zlib";
import { createHash } from "node:crypto";

export const MAX_BYTES = 512 * 1024 * 1024;
export const MAX_EXTRACTED = 2 * 1024 * 1024 * 1024;
export const MODEL_HOSTS = new Set([
  "huggingface.co",
  "cdn-lfs.huggingface.co",
  "cas-bridge.xethub.hf.co",
  "us.aws.cdn.hf.co",
]);
export const RUNTIME_HOSTS = new Set([
  "github.com",
  "release-assets.githubusercontent.com",
  "objects.githubusercontent.com",
]);
export const MODEL_BASE =
  "https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/resolve/e8f8c211226b894fcb81acc59f3b34ba3efd5f42/";
export const RUNTIME_URL =
  "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-linux-x64-1.28.0.tgz";
export const RUNTIME_ARCHIVE_SHA =
  "a3e1b79d7bb1bf09696ce675f49e4064e6c81f6202b8225624fff0e93f8d6407";
export const RUNTIME_SHA =
  "1461ef7cc3d9e49982591721683cc3e3a55580aeca9a5254e7aac47b75ee4bab";
export function safeUrl(value: string, hosts: Set<string>): URL {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error("invalid acquisition URL");
  }
  // Check original authority too: URL normalization must not admit explicit ports or credentials.
  if (
    url.protocol !== "https:" ||
    url.username ||
    url.password ||
    url.hash ||
    !hosts.has(url.hostname) ||
    value.slice(8).split(/[/?#]/)[0] !== url.hostname
  )
    throw new Error("acquisition URL is not allowed");
  return url;
}
export async function download(
  url: string,
  destination: string,
  hosts: Set<string>,
  limit: number,
  signal: AbortSignal,
  request = fetch,
): Promise<void> {
  let current = safeUrl(url, hosts);
  for (let redirects = 0; ; redirects++) {
    signal.throwIfAborted();
    let response: Response;
    try {
      response = await request(current, { redirect: "manual", signal });
    } catch {
      throw new Error("acquisition transfer failed");
    }
    if ([301, 302, 303, 307, 308].includes(response.status)) {
      await response.body?.cancel();
      if (redirects >= 5 || !response.headers.get("location"))
        throw new Error("acquisition redirect limit exceeded");
      current = safeUrl(
        new URL(response.headers.get("location")!, current).href,
        hosts,
      );
      continue;
    }
    const length = response.headers.get("content-length");
    if (
      !response.ok ||
      !response.body ||
      (length !== null && (!/^\d+$/.test(length) || Number(length) > limit))
    ) {
      await response.body?.cancel();
      throw new Error("acquisition response refused");
    }
    const fd = openSync(
      destination,
      constants.O_WRONLY |
        constants.O_CREAT |
        constants.O_EXCL |
        constants.O_NOFOLLOW,
      0o600,
    );
    let size = 0;
    try {
      for await (const chunk of response.body) {
        signal.throwIfAborted();
        size += chunk.byteLength;
        if (size > limit) throw new Error("acquisition exceeds transfer bound");
        writeFileSync(fd, chunk);
      }
      if (length !== null && size !== Number(length))
        throw new Error("truncated acquisition");
    } finally {
      closeSync(fd);
    }
    return;
  }
}
export function directoryPrefixes(
  path: string,
  paths: Pick<
    typeof nativePath,
    "resolve" | "parse" | "sep" | "join"
  > = nativePath,
): string[] {
  const absolute = paths.resolve(path);
  const root = paths.parse(absolute).root;
  if (!root) throw new Error("artifact path needs a filesystem root");
  let current = root;
  const prefixes: string[] = [root];
  for (const component of absolute
    .slice(root.length)
    .split(paths.sep)
    .filter(Boolean)) {
    current = paths.join(current, component);
    prefixes.push(current);
  }
  return prefixes;
}
export function trustedDirectory(path: string): void {
  const absolute = resolve(path);
  for (const current of directoryPrefixes(absolute)) {
    if (!existsSync(current)) {
      if (current === nativePath.parse(absolute).root)
        throw new Error("artifact filesystem root must exist");
      mkdirSync(current, { mode: 0o700 });
    }
    const stat = lstatSync(current);
    if (!stat.isDirectory() || stat.isSymbolicLink())
      throw new Error("artifact directory must not be a symlink");
  }
  const stat = lstatSync(absolute);
  if (
    process.platform !== "win32" &&
    (stat.uid !== process.getuid?.() || stat.mode & 0o022)
  )
    throw new Error(
      "artifact directory must be owned and not writable by others",
    );
}
export async function verify(
  path: string,
  expected: string,
  limit = MAX_BYTES,
): Promise<void> {
  const fd = openSync(
    path,
    constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK,
  );
  try {
    const stat = fstatSync(fd);
    if (!stat.isFile() || stat.nlink !== 1 || stat.size > limit)
      throw new Error("release asset file type or size mismatch");
    const hash = createHash("sha256");
    let size = 0;
    for await (const chunk of createReadStream(path, {
      fd,
      autoClose: false,
    })) {
      size += chunk.length;
      if (size > limit) throw new Error("release asset exceeds byte bound");
      hash.update(chunk);
    }
    const named = lstatSync(path);
    if (
      named.dev !== stat.dev ||
      named.ino !== stat.ino ||
      hash.digest("hex") !== expected
    )
      throw new Error("release asset checksum or identity mismatch");
  } finally {
    closeSync(fd);
  }
}
function existsWithoutFollowing(path: string): boolean {
  try {
    lstatSync(path);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return false;
    throw error;
  }
}
export async function acquireFile(
  path: string,
  expected: string,
  url: string,
  hosts: Set<string>,
  limit: number,
  signal: AbortSignal,
  request = fetch,
): Promise<void> {
  trustedDirectory(dirname(path));
  if (existsWithoutFollowing(path)) {
    await verify(path, expected, limit);
    return;
  }
  const temporary = mkdtempSync(join(dirname(path), ".download-"));
  try {
    const part = join(temporary, "asset");
    await download(url, part, hosts, limit, signal, request);
    await verify(part, expected, limit);
    // Never replace an existing cache entry, including one produced concurrently.
    try {
      linkSync(part, path);
      unlinkSync(part);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      await verify(path, expected, limit);
    }
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}
function field(block: Buffer, start: number, length: number): string {
  return block
    .subarray(start, start + length)
    .toString("utf8")
    .replace(/\0.*$/s, "");
}
function octal(value: string): number {
  if (!/^[0-7]+\s*$/.test(value)) throw new Error("invalid tar numeric field");
  const number = parseInt(value, 8);
  if (!Number.isSafeInteger(number) || number < 0)
    throw new Error("invalid tar size");
  return number;
}
function safeName(name: string, top: string): string {
  const trimmed = name.replace(/\/$/, "");
  if (
    !trimmed ||
    trimmed.includes("\\") ||
    trimmed
      .split("/")
      .some(
        (p) => !p || p === "." || p === ".." || /[:\x00-\x1f]|[. ]$/.test(p),
      ) ||
    (trimmed !== top && !trimmed.startsWith(`${top}/`))
  )
    throw new Error("unsafe tar path");
  return trimmed;
}
/** Parse the verified tar before writing. Reject devices, hardlinks, extension records,
 * duplicate paths, traversal and any symlink used as a directory. GNU long names
 * are unnecessary for the pinned runtime and are rejected rather than interpreted. */
export function extractTar(
  bytes: Buffer,
  destination: string,
  top = "onnxruntime-linux-x64-1.28.0",
  limit = MAX_EXTRACTED,
): void {
  if (bytes.length > limit) throw new Error("archive exceeds extraction bound");
  const entries = new Map<
    string,
    { type: string; data: Buffer; link: string }
  >();
  let offset = 0,
    ended = false;
  while (offset + 512 <= bytes.length) {
    const block = bytes.subarray(offset, offset + 512);
    offset += 512;
    if (block.every((byte) => byte === 0)) {
      if (
        bytes.length - offset < 512 ||
        !bytes.subarray(offset).every((byte) => byte === 0)
      )
        throw new Error("invalid tar terminator");
      ended = true;
      break;
    }
    const checksum = [...block].reduce(
      (sum, byte, index) => sum + (index >= 148 && index < 156 ? 32 : byte),
      0,
    );
    if (checksum !== octal(field(block, 148, 8)))
      throw new Error("tar header checksum mismatch");
    const prefix = field(block, 345, 155);
    const name = safeName(
      (prefix ? `${prefix}/` : "") + field(block, 0, 100),
      top,
    );
    const size = octal(field(block, 124, 12));
    const type = field(block, 156, 1) || "0";
    if (!["0", "2", "5"].includes(type) || (type !== "0" && size !== 0))
      throw new Error("unsupported tar entry");
    if (
      entries.has(name) ||
      entries.size >= 10000 ||
      size > MAX_EXTRACTED ||
      offset + Math.ceil(size / 512) * 512 > bytes.length
    )
      throw new Error("duplicate, oversized or truncated tar entry");
    entries.set(name, {
      type,
      data: bytes.subarray(offset, offset + size),
      link: field(block, 157, 100),
    });
    offset += Math.ceil(size / 512) * 512;
  }
  if (!ended) throw new Error("truncated tar archive");
  let materializedBytes = 0;
  for (const [name, entry] of entries) {
    let parent = posix.dirname(name);
    while (parent !== ".") {
      if (entries.has(parent) && entries.get(parent)!.type !== "5")
        throw new Error("tar parent is not a directory");
      parent = posix.dirname(parent);
    }
    if (entry.type === "2") {
      // Materialize only links resolving to regular files within the authenticated archive.
      if (posix.isAbsolute(entry.link) || entry.link.includes("\\"))
        throw new Error("unsafe tar link");
      let target = safeName(
        posix.normalize(posix.join(posix.dirname(name), entry.link)),
        top,
      );
      const seen = new Set([name]);
      while (entries.get(target)?.type === "2") {
        if (seen.has(target)) throw new Error("cyclic tar link");
        seen.add(target);
        const link = entries.get(target)!.link;
        if (posix.isAbsolute(link) || link.includes("\\"))
          throw new Error("unsafe tar link");
        target = safeName(
          posix.normalize(posix.join(posix.dirname(target), link)),
          top,
        );
      }
      if (entries.get(target)?.type !== "0")
        throw new Error("tar link target is not a regular file");
      entry.data = entries.get(target)!.data;
    }
    materializedBytes += entry.data.length;
    if (materializedBytes > limit)
      throw new Error("materialized archive exceeds extraction bound");
  }
  trustedDirectory(destination);
  if (readdirSync(destination).length)
    throw new Error("extraction directory must be empty");
  for (const [name, entry] of entries) {
    const path = join(destination, name);
    if (entry.type === "5") mkdirSync(path, { recursive: true, mode: 0o700 });
    else {
      mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
      writeFileSync(path, entry.data, { flag: "wx", mode: 0o600 });
    }
  }
}
export async function unpackRuntime(
  archive: string,
  destination: string,
  signal: AbortSignal,
  limit = MAX_EXTRACTED,
  target: DesktopTarget = "x86_64-unknown-linux-gnu",
): Promise<void> {
  const descriptor = desktopTarget(target).runtime;
  if (descriptor.origin !== "official")
    throw new Error(
      "Intel ONNX 1.28.0 requires a pinned native source build and measured provenance; no official artifact exists",
    );
  if (lstatSync(archive).size > MAX_BYTES)
    throw new Error("archive exceeds compressed bound");
  if (descriptor.archive.endsWith(".zip")) {
    extractZip(readFileSync(archive), destination, descriptor.top, limit);
    return;
  }
  const chunks: Buffer[] = [];
  let size = 0;
  const input = createReadStream(archive);
  const gzip = createGunzip();
  input.on("error", (error) => gzip.destroy(error));
  const abort = () => {
    input.destroy();
    gzip.destroy(new Error("acquisition deadline exceeded"));
  };
  signal.addEventListener("abort", abort, { once: true });
  try {
    for await (const chunk of input.pipe(gzip)) {
      signal.throwIfAborted();
      size += chunk.length;
      if (size > limit) throw new Error("archive exceeds extraction bound");
      chunks.push(chunk);
    }
    extractTar(Buffer.concat(chunks), destination, descriptor.top, limit);
  } finally {
    signal.removeEventListener("abort", abort);
    input.destroy();
    gzip.destroy();
  }
}
export async function stage(
  root: string,
  target: DesktopTarget = "x86_64-unknown-linux-gnu",
  inputs?: { runtimeArchive?: string; modelDirectory?: string },
): Promise<Record<string, string>> {
  const descriptor = desktopTarget(target).runtime;
  if (descriptor.origin !== "official")
    throw new Error(
      "Intel ONNX 1.28.0 requires a pinned native source build and measured provenance; no official artifact exists",
    );
  root = realpathSync(root);
  const signal = AbortSignal.timeout(300_000);
  const cache = join(root, "qualification/.artifacts/models");
  trustedDirectory(cache);
  const model =
    inputs?.modelDirectory ??
    join(cache, "paraphrase-multilingual-MiniLM-L12-v2");
  if (inputs?.modelDirectory) {
    for (const [name, pin] of Object.entries(MODEL_PINS))
      await verify(join(model, name), pin);
  } else {
    await acquireFile(
      join(model, "model.onnx"),
      MODEL_PINS["model.onnx"],
      `${MODEL_BASE}onnx/model.onnx`,
      MODEL_HOSTS,
      MAX_BYTES,
      signal,
    );
    for (const [asset, source] of Object.entries({
      "tokenizer.json": "multilingual-minilm-tokenizer.json",
      LICENSE: "multilingual-minilm-tokenizer.LICENSE",
    })) {
      const pin = MODEL_PINS[asset as keyof typeof MODEL_PINS];
      const fixture = join(root, "crates/hieronymus/tests/fixtures", source);
      await verify(fixture, pin);
      const destination = join(model, asset);
      if (!existsSync(destination))
        writeFileSync(destination, readFileSync(fixture), {
          flag: "wx",
          mode: 0o600,
        });
      await verify(destination, pin);
    }
    await acquireFile(
      join(model, "README.md"),
      MODEL_PINS["README.md"],
      `${MODEL_BASE}README.md`,
      MODEL_HOSTS,
      65536,
      signal,
    );
  }
  const archive = inputs?.runtimeArchive ?? join(cache, descriptor.archive);
  if (inputs?.runtimeArchive)
    await verify(archive, descriptor.sha256, descriptor.size);
  else
    await acquireFile(
      archive,
      descriptor.sha256,
      descriptor.url,
      RUNTIME_HOSTS,
      MAX_BYTES,
      signal,
    );
  // Always extract the hash-verified archive into a fresh directory. Cached runtime
  // trees cannot supply unverified notices or a changed library via symlinks.
  const extraction = mkdtempSync(join(cache, ".release-runtime-"));
  try {
    await unpackRuntime(archive, extraction, signal, MAX_EXTRACTED, target);
    const runtime = join(extraction, descriptor.top);
    const library = join(runtime, descriptor.member);
    for (const [name, pin] of Object.entries(descriptor.members))
      await verify(join(runtime, name), pin.sha256, pin.size);
    for (const notice of [
      "LICENSE",
      "ThirdPartyNotices.txt",
      "VERSION_NUMBER",
    ]) {
      const stat = lstatSync(join(runtime, notice));
      if (!stat.isFile() || stat.isSymbolicLink() || stat.size === 0)
        throw new Error("missing regular runtime notice");
    }
    if (
      readFileSync(join(runtime, "VERSION_NUMBER"), "utf8").trim() !== "1.28.0"
    )
      throw new Error("runtime version mismatch");
    signal.throwIfAborted();
    return {
      HIERO_RELEASE_ONNX_RUNTIME: library,
      HIERO_RELEASE_ONNX_SHA256: descriptor.members[descriptor.member].sha256,
      HIERO_RELEASE_RUNTIME_DIR: runtime,
      HIERO_RELEASE_TARGET: target,
      HIERO_RELEASE_MODEL_DIR: model,
    };
  } catch (error) {
    rmSync(extraction, { recursive: true, force: true });
    throw error;
  }
}

export function extractZip(
  bytes: Buffer,
  destination: string,
  top: string,
  limit = MAX_EXTRACTED,
): void {
  if (bytes.length > MAX_BYTES) throw new Error("ZIP exceeds compressed bound");
  const entries = zipMembers(bytes, limit);
  for (const name of entries.keys()) safeName(name, top);
  trustedDirectory(destination);
  if (readdirSync(destination).length)
    throw new Error("extraction directory must be empty");
  for (const [name, data] of entries) {
    const path = join(destination, name);
    if (data === null) mkdirSync(path, { recursive: true, mode: 0o700 });
    else {
      mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
      writeFileSync(path, data, { flag: "wx", mode: 0o600 });
    }
  }
}
