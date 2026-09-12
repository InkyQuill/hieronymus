import { expect, test } from "bun:test";
import { desktopTarget, type OfficialRuntime } from "./desktop-targets";
import {
  downloadRetainedArtifact,
  reviewedSource,
  validateSourceReceipt,
} from "./reviewed-runtime";
import { INTEL_SOURCE } from "./build-intel-runtime";

test("retained artifact download terminates a stalled process and rejects failure", () => {
  expect(() =>
    downloadRetainedArtifact(
      [process.execPath, "-e", "setInterval(() => {}, 1000)"],
      100,
    ),
  ).toThrow("Could not download");
  expect(() =>
    downloadRetainedArtifact([process.execPath, "-e", "process.exit(1)"]),
  ).toThrow("Could not download");
});

function fixture() {
  const runtime = structuredClone(
    desktopTarget("x86_64-unknown-linux-gnu").runtime,
  ) as OfficialRuntime;
  runtime.origin = "source-reviewed";
  runtime.top = "onnxruntime-osx-x86_64-1.28.0-source";
  runtime.archive = `${runtime.top}.tgz`;
  runtime.members["lib/libonnxruntime.dylib"] = runtime.members[runtime.member];
  delete runtime.members[runtime.member];
  runtime.member = "lib/libonnxruntime.dylib";
  runtime.url = `https://github.com/InkyQuill/hieronymus/releases/download/v0.9.0/${runtime.archive}`;
  runtime.source = {
    repository: "https://github.com/microsoft/onnxruntime.git",
    revision: INTEL_SOURCE,
    run: "123",
    receipt_sha256: "a".repeat(64),
  };
  const receipt = {
    format_version: 1,
    target: "x86_64-apple-darwin",
    architecture: "x86_64",
    runtime_version: "1.28.0",
    source_repository: runtime.source.repository,
    source_revision: INTEL_SOURCE,
    archive: runtime.archive,
    sha256: runtime.sha256,
    members: structuredClone(runtime.members),
  };
  return { runtime, receipt };
}
test("reviewed source receipt must match every compiled archive and member pin", () => {
  const f = fixture();
  expect(reviewedSource(f.runtime, "x86_64-apple-darwin").run).toBe("123");
  validateSourceReceipt(f.receipt, f.runtime);
  f.receipt.members[Object.keys(f.receipt.members)[0]].sha256 = "f".repeat(64);
  expect(() => validateSourceReceipt(f.receipt, f.runtime)).toThrow(
    "member mismatch",
  );
});
test("source revision, build target, and acquisition origin cannot be substituted", () => {
  const f = fixture();
  expect(() => reviewedSource(f.runtime, "aarch64-apple-darwin")).toThrow(
    "provenance",
  );
  f.receipt.source_revision = "f".repeat(40);
  expect(() => validateSourceReceipt(f.receipt, f.runtime)).toThrow(
    "compiled pins",
  );
  f.runtime.url = `https://foreign.invalid/${f.runtime.archive}`;
  expect(() => reviewedSource(f.runtime, "x86_64-apple-darwin")).toThrow(
    "provenance",
  );
});
test("reviewed source authority requires bounded run and receipt digest", () => {
  const f = fixture();
  f.runtime.source!.run = "--repo=foreign";
  expect(() => reviewedSource(f.runtime, "x86_64-apple-darwin")).toThrow(
    "provenance",
  );
  f.runtime.source!.run = "123";
  f.runtime.source!.receipt_sha256 = "missing";
  expect(() => reviewedSource(f.runtime, "x86_64-apple-darwin")).toThrow(
    "provenance",
  );
});
