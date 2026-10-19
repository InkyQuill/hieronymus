import { expect, test } from "bun:test";
import { mkdtempSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { acquirePublishedModel, publishedModel } from "./published-model";

test("a changed model pin is rejected before network acquisition", async () => {
  let requests = 0;
  const request = (async () => {
    requests++;
    return new Response("corrupt");
  }) as typeof fetch;
  await expect(
    acquirePublishedModel(
      "unused",
      { ...publishedModel, sha256: "0".repeat(64) },
      request,
    ),
  ).rejects.toThrow("pin the new model first");
  expect(requests).toBe(0);
});

test("a corrupt published model is never installed as the shared archive", async () => {
  const directory = mkdtempSync(join(tmpdir(), "published-model-"));
  try {
    const request = (async () => new Response("corrupt")) as typeof fetch;
    await expect(
      acquirePublishedModel(directory, publishedModel, request),
    ).rejects.toThrow("checksum");
    expect(existsSync(join(directory, publishedModel.archive))).toBe(false);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
