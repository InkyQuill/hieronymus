/** One common, deterministic model archive; acquisition cache is never runtime authority. */
import {
  createReadStream,
  createWriteStream,
  existsSync,
  linkSync,
  mkdtempSync,
  rmSync,
  unlinkSync,
} from "node:fs";
import { join } from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { createGzip } from "node:zlib";
import { createHash } from "node:crypto";
import {
  MODEL_NAME,
  MODEL_REVISION,
  MODEL_PINS,
  modelArtifactName,
  modelMembers,
  parseReleaseV2,
  platformArtifactName,
  type ModelArtifact,
} from "./desktop-targets";
import {
  acquireFile,
  verify,
  trustedDirectory,
  MAX_BYTES,
} from "./release-assets";
import { digest } from "./check-rust-release";
function validateModel(model: ModelArtifact): void {
  parseReleaseV2(
    JSON.stringify({
      format_version: 2,
      version: "0.0.0",
      target: "x86_64-unknown-linux-gnu",
      channel: "stable",
      platform: {
        archive: platformArtifactName("0.0.0", "x86_64-unknown-linux-gnu"),
        sha256: "a".repeat(64),
      },
      model,
      signature: null,
    }),
    "x86_64-unknown-linux-gnu",
  );
}
export async function acquireCommonModel(
  model: ModelArtifact,
  cache: string,
  url: string,
  hosts: Set<string>,
  signal: AbortSignal,
  request = fetch,
): Promise<string> {
  validateModel(model);
  const destination = join(cache, `${model.sha256}.tar.gz`);
  await acquireFile(
    destination,
    model.sha256,
    url,
    hosts,
    MAX_BYTES,
    signal,
    request,
  );
  return destination;
}
export async function packageCommonModel(
  modelDirectory: string,
  outputDirectory: string,
): Promise<ModelArtifact> {
  trustedDirectory(outputDirectory);
  const temp = mkdtempSync(join(outputDirectory, ".common-model-"));
  try {
    async function* tar() {
      for (const [name, pin] of Object.entries(MODEL_PINS)) {
        const path = join(modelDirectory, name);
        await verify(path, pin);
        const { stat } = await import("node:fs/promises");
        const size = (await stat(path)).size;
        const header = Buffer.alloc(512);
        header.write(`models/minilm/${name}`);
        header.write("0000600\0", 100);
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
        const hash = createHash("sha256");
        let count = 0;
        for await (const chunk of createReadStream(path)) {
          count += chunk.length;
          if (count > size) throw new Error("model changed during packaging");
          hash.update(chunk);
          yield chunk;
        }
        if (count !== size || hash.digest("hex") !== pin)
          throw new Error("model changed during packaging");
        yield Buffer.alloc((512 - (size % 512)) % 512);
      }
      yield Buffer.alloc(1024);
    }
    const part = join(temp, "model.tar.gz");
    await pipeline(
      Readable.from(tar()),
      createGzip({ level: 6 }),
      createWriteStream(part, { flags: "wx", mode: 0o600 }),
    );
    const sha256 = await digest(part);
    await verify(part, sha256);
    const archive = modelArtifactName();
    const destination = join(outputDirectory, archive);
    if (existsSync(destination)) await verify(destination, sha256);
    else {
      linkSync(part, destination);
      unlinkSync(part);
    }
    return {
      name: MODEL_NAME,
      revision: MODEL_REVISION,
      archive,
      sha256,
      members: modelMembers(),
    };
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
}
