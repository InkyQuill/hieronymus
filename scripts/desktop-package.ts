/** Native packaging of exact-target prebuilt binaries. Shared model bytes are never recompressed. */
import {
  constants,
  createReadStream,
  createWriteStream,
  readFileSync,
  writeFileSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  copyFileSync,
  lstatSync,
  existsSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { createHash } from "node:crypto";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { createGzip, deflateRawSync } from "node:zlib";
import {
  desktopTarget,
  modelMembers,
  parseReleaseV2,
  platformArtifactName,
  type DesktopTarget,
  type ModelArtifact,
} from "./desktop-targets";
import { digest } from "./check-rust-release";
import { verify, trustedDirectory } from "./release-assets";

export interface Options {
  target: DesktopTarget;
  input: string;
  runtime: string;
  common: string;
  out: string;
}
const exeNames = (target: DesktopTarget) =>
  target.includes("windows")
    ? ["hiero.exe", "hiero-desktop.exe", "hiero-launcher.exe"]
    : ["hiero", "hiero-desktop"];
function regular(path: string) {
  if (!lstatSync(path).isFile())
    throw new Error(`required regular file: ${path}`);
}
export async function validateInputs(o: Options): Promise<ModelArtifact> {
  const target = desktopTarget(o.target);
  if (target.runtime.origin !== "official")
    throw new Error(
      "Intel runtime requires reviewed source-build promotion; no package can be emitted",
    );
  for (const name of exeNames(o.target)) regular(join(o.input, name));
  for (const name of Object.keys(target.runtime.members))
    regular(join(o.runtime, name));
  regular(join(o.common, "common-model.json"));
  const model = JSON.parse(
    readFileSync(join(o.common, "common-model.json"), "utf8"),
  );
  parseReleaseV2(
    JSON.stringify({
      format_version: 2,
      version: "0.0.0",
      target: o.target,
      channel: "dev",
      platform: {
        archive: platformArtifactName("0.0.0", o.target),
        sha256: "0".repeat(64),
      },
      model,
      signature: null,
    }),
    o.target,
  );
  await verify(join(o.common, model.archive), model.sha256);
  for (const [name, pin] of Object.entries(target.runtime.members))
    await verify(join(o.runtime, name), pin.sha256);
  return model;
}
export function validateBinary(bytes: Buffer, target: DesktopTarget) {
  let actual = "";
  if (
    bytes.length >= 64 &&
    bytes.subarray(0, 4).equals(Buffer.from([127, 69, 76, 70])) &&
    bytes[4] === 2 &&
    bytes[5] === 1
  )
    actual = bytes.readUInt16LE(18) === 62 ? "x86_64-unknown-linux-gnu" : "";
  if (bytes.length >= 64 && bytes.toString("ascii", 0, 2) === "MZ") {
    const pe = bytes.readUInt32LE(60);
    if (
      pe + 24 <= bytes.length &&
      bytes.readUInt32LE(pe) === 0x4550 &&
      bytes.readUInt16LE(pe + 4) === 0x8664
    )
      actual = "x86_64-pc-windows-msvc";
  }
  if (bytes.length >= 32 && bytes.readUInt32LE(0) === 0xfeedfacf)
    actual =
      bytes.readUInt32LE(4) === 0x0100000c
        ? "aarch64-apple-darwin"
        : bytes.readUInt32LE(4) === 0x01000007
          ? "x86_64-apple-darwin"
          : "";
  if (actual !== target)
    throw new Error(`executable architecture/format mismatch for ${target}`);
}
export function workspaceVersion(source: string): string {
  const version = (Bun.TOML.parse(source) as any).workspace?.package?.version;
  if (typeof version !== "string")
    throw new Error("Missing workspace package version");
  return version;
}
export function validateIdentity(
  identity: any,
  version: string,
  target: DesktopTarget,
) {
  if (identity.version !== version)
    throw new Error("prebuilt executable version differs from workspace");
  if (identity.target !== undefined && identity.target !== target)
    throw new Error("prebuilt executable target differs from package");
}
function command(args: string[]) {
  const result = Bun.spawnSync(args, { stdout: "pipe", stderr: "pipe" });
  if (result.exitCode !== 0)
    throw new Error(`${args[0]} failed: ${result.stderr.toString()}`);
  return result.stdout.toString();
}
const crcTable = Array.from({ length: 256 }, (_, i) => {
  let c = i;
  for (let j = 0; j < 8; j++) c = (c >>> 1) ^ (c & 1 ? 0xedb88320 : 0);
  return c >>> 0;
});
function crc32(bytes: Buffer) {
  let c = 0xffffffff;
  for (const b of bytes) c = (c >>> 8) ^ crcTable[(c ^ b) & 255];
  return (c ^ 0xffffffff) >>> 0;
}
/** Strict contiguous ZIP32: no descriptors, extras, comments, ZIP64 or alias names. */
export function zip32(members: { name: string; bytes: Buffer }[]): Buffer {
  const local: Buffer[] = [];
  const central: Buffer[] = [];
  let offset = 0;
  const names = new Set<string>();
  for (const { name, bytes } of members) {
    if (
      !/^[A-Za-z0-9_.\/-]+$/.test(name) ||
      name.split("/").some((p) => !p || p === "." || p === "..") ||
      names.has(name)
    )
      throw new Error("invalid ZIP member");
    names.add(name);
    const filename = Buffer.from(name),
      compressed = deflateRawSync(bytes),
      crc = crc32(bytes);
    const l = Buffer.alloc(30);
    l.writeUInt32LE(0x04034b50);
    l.writeUInt16LE(20, 4);
    l.writeUInt16LE(8, 8);
    l.writeUInt32LE(crc, 14);
    l.writeUInt32LE(compressed.length, 18);
    l.writeUInt32LE(bytes.length, 22);
    l.writeUInt16LE(filename.length, 26);
    const c = Buffer.alloc(46);
    c.writeUInt32LE(0x02014b50);
    c.writeUInt16LE(20, 4);
    c.writeUInt16LE(20, 6);
    c.writeUInt16LE(8, 10);
    c.writeUInt32LE(crc, 16);
    c.writeUInt32LE(compressed.length, 20);
    c.writeUInt32LE(bytes.length, 24);
    c.writeUInt16LE(filename.length, 28);
    c.writeUInt32LE(offset, 42);
    local.push(l, filename, compressed);
    central.push(c, filename);
    offset += l.length + filename.length + compressed.length;
    if (offset >= 0xffffffff || members.length >= 65535)
      throw new Error("ZIP32 bound exceeded");
  }
  const dir = Buffer.concat(central),
    end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50);
  end.writeUInt16LE(members.length, 8);
  end.writeUInt16LE(members.length, 10);
  end.writeUInt32LE(dir.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...local, dir, end]);
}
async function tar(root: string, names: string[], out: string) {
  async function* records() {
    for (const name of names) {
      const path = join(root, name);
      const size = lstatSync(path).size;
      const header = Buffer.alloc(512);
      if (Buffer.byteLength(name) > 100) throw new Error("ustar name bound");
      header.write(name);
      header.write(
        name.endsWith("hiero") || name.endsWith("hiero-desktop")
          ? "0000755\0"
          : "0000644\0",
        100,
      );
      header.write("0000000\0", 108);
      header.write("0000000\0", 116);
      header.write(size.toString(8).padStart(11, "0") + "\0", 124);
      header.write("00000000000\0", 136);
      header.fill(32, 148, 156);
      header[156] = 48;
      header.write("ustar\0", 257);
      header.write("00", 263);
      header.write(
        [...header]
          .reduce((a, b) => a + b, 0)
          .toString(8)
          .padStart(6, "0") + "\0 ",
        148,
      );
      yield header;
      for await (const chunk of createReadStream(path)) yield chunk;
      yield Buffer.alloc((512 - (size % 512)) % 512);
    }
    yield Buffer.alloc(1024);
  }
  await pipeline(
    Readable.from(records()),
    createGzip({ level: 6 }),
    createWriteStream(out, { flags: "wx", mode: 0o600 }),
  );
}
/** A prior complete or interrupted publication is immutable, including dangling aliases. */
export function assertOutputAvailable(out: string, names: string[]) {
  for (const name of names) {
    try {
      lstatSync(join(out, name));
    } catch (error: any) {
      if (error.code === "ENOENT") continue;
      throw error;
    }
    throw new Error(`immutable output already exists: ${name}`);
  }
}
export async function packageDesktop(o: Options) {
  const model = await validateInputs(o);
  const target = desktopTarget(o.target);
  const host =
    process.platform === "win32"
      ? "x86_64-pc-windows-msvc"
      : process.platform === "darwin"
        ? process.arch === "arm64"
          ? "aarch64-apple-darwin"
          : "x86_64-apple-darwin"
        : "x86_64-unknown-linux-gnu";
  if (host !== o.target)
    throw new Error(
      "package on the exact native target to verify executable versions and native strip tooling",
    );
  const version = workspaceVersion(
    readFileSync(new URL("../Cargo.toml", import.meta.url), "utf8"),
  );
  for (const name of exeNames(o.target))
    validateBinary(readFileSync(join(o.input, name)), o.target);
  for (const name of exeNames(o.target).filter((n) => !n.includes("launcher")))
    validateIdentity(
      JSON.parse(command([join(o.input, name), "version", "--json"])),
      version,
      o.target,
    );
  trustedDirectory(o.out);
  const reservation = join(o.out, ".desktop-package.lock");
  mkdirSync(reservation, { mode: 0o700 });
  let work = "";
  try {
    assertOutputAvailable(o.out, [
      target.metadata,
      platformArtifactName(version, o.target),
      `evidence-${o.target}.json`,
      `assets-${o.target}.json`,
      `install-desktop-${o.target}.${o.target.includes("windows") ? "ps1" : "sh"}`,
    ]);
    work = mkdtempSync(join(o.out, ".package-"));
    const payload = join(work, "payload");
    mkdirSync(payload);
    const names: string[] = [];
    const symbols: any[] = [];
    function copy(source: string, name: string) {
      mkdirSync(join(payload, name, ".."), { recursive: true });
      copyFileSync(source, join(payload, name));
      names.push(name);
    }
    for (const name of exeNames(o.target)) {
      const member =
        o.target.includes("apple") && name === "hiero-desktop"
          ? "Hieronymus.app/Contents/MacOS/hiero-desktop"
          : name;
      copy(join(o.input, name), member);
      const path = join(payload, member);
      const before = lstatSync(path).size;
      if (process.platform === "linux") {
        const original = await digest(path),
          debug = join(work, `${name}-${original}.debug`);
        command(["objcopy", "--only-keep-debug", path, debug]);
        command(["strip", "--strip-unneeded", path]);
        symbols.push({
          binary: member,
          original_sha256: original,
          debug: debug.split("/").pop(),
          before,
          after: lstatSync(path).size,
        });
      } else if (process.platform === "darwin") {
        command(["strip", "-S", path]);
        symbols.push({
          binary: member,
          before,
          after: lstatSync(path).size,
          debug: "not separately generated",
        });
      } else
        symbols.push({
          binary: member,
          before,
          after: before,
          debug:
            "MSVC release executable retained; supply matching PDB separately when generated",
        });
    }
    if (target.runtime.origin !== "official")
      throw new Error("unpromoted runtime");
    for (const name of Object.keys(target.runtime.members))
      copy(
        join(o.runtime, name),
        name.startsWith("lib/") ? name : `licenses/runtime/${name}`,
      );
    if (o.target.includes("apple")) {
      const plist = readFileSync(
        new URL("../assets/macos/Info.plist", import.meta.url),
        "utf8",
      ).replace(
        "</dict>",
        `<key>CFBundleShortVersionString</key><string>${version}</string><key>CFBundleVersion</key><string>${version}</string></dict>`,
      );
      writeFileSync(join(payload, "Hieronymus.app/Contents/Info.plist"), plist);
      names.push("Hieronymus.app/Contents/Info.plist");
      const iconset = resolve(
        process.env.HIERO_RELEASE_ICONSET ??
          "target/desktop-icons/app/hieronymus.iconset",
      );
      mkdirSync(join(payload, "Hieronymus.app/Contents/Resources"), {
        recursive: true,
      });
      command([
        "iconutil",
        "-c",
        "icns",
        iconset,
        "-o",
        join(payload, "Hieronymus.app/Contents/Resources/hieronymus.icns"),
      ]);
      names.push("Hieronymus.app/Contents/Resources/hieronymus.icns");
    }
    const sha256: Record<string, string> = { ...modelMembers() };
    for (const name of names) sha256[name] = await digest(join(payload, name));
    writeFileSync(
      join(payload, "assets.json"),
      JSON.stringify(
        {
          model: model.name,
          revision: model.revision,
          runtime_version: "1.28.0",
          target: o.target,
          sha256,
        },
        null,
        2,
      ) + "\n",
    );
    names.push("assets.json");
    const archive = platformArtifactName(version, o.target),
      part = join(work, archive);
    if (o.target.includes("windows"))
      writeFileSync(
        part,
        zip32(
          names.map((name) => ({
            name,
            bytes: readFileSync(join(payload, name)),
          })),
        ),
      );
    else await tar(payload, names, part);
    const metadata = parseReleaseV2(
      JSON.stringify({
        format_version: 2,
        version,
        target: o.target,
        channel: process.env.HIERONYMUS_RELEASE_CHANNEL ?? "dev",
        platform: { archive, sha256: await digest(part) },
        model,
        signature: null,
      }),
      o.target,
    );
    // Verify complete pair with the exact native CLI before publishing success metadata.
    copyFileSync(join(o.common, model.archive), join(work, model.archive));
    writeFileSync(
      join(work, target.metadata),
      JSON.stringify(metadata, null, 2) + "\n",
    );
    command([
      join(o.input, target.executable),
      "release-verify",
      "--release-dir",
      work,
    ]);
    const shared = join(o.out, model.archive);
    if (existsSync(shared)) await verify(shared, model.sha256);
    else
      copyFileSync(
        join(o.common, model.archive),
        shared,
        constants.COPYFILE_EXCL,
      );
    if (!o.target.includes("windows")) {
      for (const name of ["desktop-metadata.awk", "install.sh"]) {
        const destination = join(o.out, name),
          source = new URL(`./${name}`, import.meta.url);
        if (
          existsSync(destination) &&
          readFileSync(destination, "utf8") !== readFileSync(source, "utf8")
        )
          throw new Error("shared installer filename collision");
        if (!existsSync(destination))
          copyFileSync(source, destination, constants.COPYFILE_EXCL);
      }
    }
    assertOutputAvailable(
      o.out,
      symbols.filter((s) => s.original_sha256).map((s) => s.debug),
    );
    copyFileSync(part, join(o.out, archive), constants.COPYFILE_EXCL);
    for (const symbol of symbols)
      if (symbol.original_sha256)
        copyFileSync(
          join(work, symbol.debug),
          join(o.out, symbol.debug),
          constants.COPYFILE_EXCL,
        );
    copyFileSync(
      new URL(
        o.target.includes("windows")
          ? "./install-desktop.ps1"
          : "./install-desktop.sh",
        import.meta.url,
      ),
      join(
        o.out,
        o.target.includes("windows")
          ? `install-desktop-${o.target}.ps1`
          : `install-desktop-${o.target}.sh`,
      ),
      constants.COPYFILE_EXCL,
    );
    copyFileSync(
      join(payload, "assets.json"),
      join(o.out, `assets-${o.target}.json`),
      constants.COPYFILE_EXCL,
    );
    writeFileSync(
      join(o.out, `evidence-${o.target}.json`),
      JSON.stringify(
        {
          version,
          target: o.target,
          signing: "unsigned; no signing identity or notarization supplied",
          metadata_sha256: await digest(join(work, target.metadata)),
          platform_sha256: metadata.platform.sha256,
          model_sha256: model.sha256,
          assets_sha256: await digest(join(payload, "assets.json")),
          symbols,
        },
        null,
        2,
      ) + "\n",
      { flag: "wx", mode: 0o600 },
    );
    // The unique success manifest is the final publication step.
    copyFileSync(
      join(work, target.metadata),
      join(o.out, target.metadata),
      constants.COPYFILE_EXCL,
    );
    return metadata;
  } finally {
    if (work) rmSync(work, { recursive: true, force: true });
    rmSync(reservation, { recursive: true });
  }
}
if (import.meta.main) {
  const args = new Map<string, string>();
  for (let i = 2; i < process.argv.length; i += 2) {
    const key = process.argv[i],
      value = process.argv[i + 1];
    if (
      ![
        "--target",
        "--out",
        "--input",
        "--runtime-dir",
        "--common-model-dir",
      ].includes(key) ||
      !value ||
      args.has(key)
    )
      throw new Error(
        "usage: desktop-package.ts --target TRIPLE --out DIR [--input DIR --runtime-dir DIR --common-model-dir DIR]",
      );
    args.set(key, value);
  }
  const target = desktopTarget(args.get("--target") ?? "").target;
  const out = args.get("--out");
  if (!out) throw new Error("--out required");
  await packageDesktop({
    target,
    out: resolve(out),
    input: resolve(
      args.get("--input") ??
        `${process.env.CARGO_TARGET_DIR ?? "target"}/${target}/release`,
    ),
    runtime: resolve(
      args.get("--runtime-dir") ??
        process.env.HIERO_RELEASE_RUNTIME_DIR ??
        "target/release-runtime",
    ),
    common: resolve(
      args.get("--common-model-dir") ??
        process.env.HIERO_COMMON_MODEL_DIR ??
        "target/common-model",
    ),
  });
}
