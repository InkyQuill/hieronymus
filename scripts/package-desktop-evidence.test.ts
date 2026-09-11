import { expect, test } from "bun:test";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { evidenceFiles } from "./package-desktop-evidence";
test("evidence ingestion allowlists bounded records and captures", () => {
  const root = mkdtempSync(join(tmpdir(), "evidence-ingestion-"));
  try {
    const names = Array.from({ length: 7 }, (_, i) => `record-${i}.json`);
    writeFileSync(join(root, "records.json"), JSON.stringify(names));
    for (const name of names)
      writeFileSync(
        join(root, name),
        JSON.stringify({ checks: [{ evidence_path: "capture.txt" }] }),
      );
    writeFileSync(join(root, "capture.txt"), "native observations");
    expect(evidenceFiles(root).length).toBe(9);
    writeFileSync(join(root, "surprise.exe"), "unexpected");
    expect(() => evidenceFiles(root)).toThrow("unknown evidence attachment");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
