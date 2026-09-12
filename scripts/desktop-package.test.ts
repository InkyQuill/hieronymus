import { test, expect } from "bun:test";
import {
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  existsSync,
  readFileSync,
  symlinkSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { desktopTarget } from "./desktop-targets";
import {
  validateBinary,
  validateInputs,
  validateIdentity,
  zip32,
  assertOutputAvailable,
  workspaceVersion,
} from "./desktop-package";

test("platform metadata cannot collide", () => {
  const names = [
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
  ].map((t) => desktopTarget(t).metadata);
  expect(new Set(names).size).toBe(4);
  expect(names).not.toContain("release.json");
});
test("wrong executable architecture fails before execution", () => {
  const elf = Buffer.alloc(64);
  elf.write("\x7fELF");
  elf[4] = 2;
  elf[5] = 1;
  elf.writeUInt16LE(183, 18);
  expect(() => validateBinary(elf, "x86_64-unknown-linux-gnu")).toThrow(
    "architecture",
  );
});
test("stale executable version is refused", () => {
  expect(() =>
    validateIdentity(
      { version: "0.8.0", target: "x86_64-unknown-linux-gnu" },
      "0.9.0",
      "x86_64-unknown-linux-gnu",
    ),
  ).toThrow("version");
});
test("missing helper, Windows DLL and model fail without success metadata", async () => {
  const root = mkdtempSync(join(tmpdir(), "desktop-package-"));
  try {
    const options = {
      target: "x86_64-pc-windows-msvc" as const,
      input: root,
      runtime: root,
      common: root,
      out: join(root, "out"),
    };
    await expect(validateInputs(options)).rejects.toThrow("hiero.exe");
    writeFileSync(join(root, "hiero.exe"), "binary");
    await expect(validateInputs(options)).rejects.toThrow("hiero-desktop.exe");
    writeFileSync(join(root, "hiero-desktop.exe"), "helper");
    writeFileSync(join(root, "hiero-launcher.exe"), "launcher");
    mkdirSync(join(root, "lib"));
    writeFileSync(join(root, "lib/libonnxruntime.so"), "wrong runtime");
    await expect(validateInputs(options)).rejects.toThrow("onnxruntime.dll");
    expect(
      existsSync(join(options.out, desktopTarget(options.target).metadata)),
    ).toBe(false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
test("ZIP32 producer has matching headers without descriptors or extras", () => {
  const data = zip32([{ name: "hiero.exe", bytes: Buffer.from("payload") }]);
  expect(data.readUInt32LE(0)).toBe(0x04034b50);
  expect(data.readUInt16LE(6)).toBe(0);
  expect(data.readUInt16LE(28)).toBe(0);
  const central = data.indexOf(Buffer.from([0x50, 0x4b, 0x01, 0x02]));
  expect(data.readUInt32LE(14)).toBe(data.readUInt32LE(central + 16));
  expect(data.readUInt32LE(18)).toBe(data.readUInt32LE(central + 20));
});

test("missing model and mismatched archive digest fail before metadata", async () => {
  const { modelArtifactName, modelMembers, MODEL_NAME, MODEL_REVISION } =
    await import("./desktop-targets");
  const root = mkdtempSync(join(tmpdir(), "desktop-model-"));
  try {
    const target = "x86_64-unknown-linux-gnu" as const,
      descriptor = desktopTarget(target);
    for (const name of [
      "hiero",
      "hiero-desktop",
      ...Object.keys((descriptor.runtime as any).members),
    ]) {
      mkdirSync(join(root, name, ".."), { recursive: true });
      writeFileSync(join(root, name), "unqualified bytes");
    }
    const options = {
      target,
      input: root,
      runtime: root,
      common: root,
      out: join(root, "out"),
    };
    await expect(validateInputs(options)).rejects.toThrow("common-model.json");
    writeFileSync(
      join(root, "common-model.json"),
      JSON.stringify({
        archive: modelArtifactName(),
        sha256: "a".repeat(64),
        name: MODEL_NAME,
        revision: MODEL_REVISION,
        members: modelMembers(),
      }),
    );
    writeFileSync(join(root, modelArtifactName()), "corrupt model archive");
    await expect(validateInputs(options)).rejects.toThrow();
    expect(existsSync(join(options.out, descriptor.metadata))).toBe(false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("bootstrap strict parser accepts canonical metadata and rejects duplicate/path/unknown keys", async () => {
  const {
    modelArtifactName,
    modelMembers,
    MODEL_NAME,
    MODEL_REVISION,
    platformArtifactName,
  } = await import("./desktop-targets");
  const target = "x86_64-unknown-linux-gnu" as const;
  const m = {
    format_version: 2,
    version: "0.9.0",
    target,
    channel: "dev",
    signature: null,
    platform: {
      archive: platformArtifactName("0.9.0", target),
      sha256: "a".repeat(64),
    },
    model: {
      archive: modelArtifactName(),
      sha256: "b".repeat(64),
      name: MODEL_NAME,
      revision: MODEL_REVISION,
      members: modelMembers(),
    },
  };
  const parse = (text: string) =>
    Bun.spawnSync(
      ["awk", "-v", `target=${target}`, "-f", "scripts/desktop-metadata.awk"],
      { stdin: Buffer.from(text), stdout: "pipe", stderr: "pipe" },
    );
  expect(parse(JSON.stringify(m)).exitCode).toBe(0);
  expect(
    parse(
      JSON.stringify(m).replace(
        '"format_version":2',
        '"format_version":2,"format_version":2',
      ),
    ).exitCode,
  ).not.toBe(0);
  expect(parse(JSON.stringify({ ...m, extra: true })).exitCode).not.toBe(0);
  expect(
    parse(
      JSON.stringify({
        ...m,
        platform: { ...m.platform, archive: "../hiero.tar.gz" },
      }),
    ).exitCode,
  ).not.toBe(0);
});

test("repeated or interrupted publication refuses existing output and preserves bytes", () => {
  const out = mkdtempSync(join(tmpdir(), "desktop-publication-"));
  try {
    writeFileSync(join(out, "platform.tar.gz"), "interrupted archive");
    expect(() =>
      assertOutputAvailable(out, ["release.json", "platform.tar.gz"]),
    ).toThrow("immutable");
    expect(readFileSync(join(out, "platform.tar.gz"), "utf8")).toBe(
      "interrupted archive",
    );
    writeFileSync(join(out, "release.json"), "prior successful identity");
    expect(() => assertOutputAvailable(out, ["release.json"])).toThrow(
      "immutable",
    );
    expect(readFileSync(join(out, "release.json"), "utf8")).toBe(
      "prior successful identity",
    );
    if (process.platform !== "win32") {
      symlinkSync("missing", join(out, "dangling"));
      expect(() => assertOutputAvailable(out, ["dangling"])).toThrow(
        "immutable",
      );
    }
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
});

test("workspace identity does not mistake rust-version for package version", () => {
  expect(
    workspaceVersion(
      '[workspace.package]\nrust-version = "1.96"\nversion = "0.9.0"',
    ),
  ).toBe("0.9.0");
});
