/** Reuse the canonical archive already published with v0.13.4, bound to its bytes. */
import { join } from "node:path";
import { writeFileSync } from "node:fs";
import {
  MODEL_NAME,
  MODEL_REVISION,
  modelArtifactName,
  modelMembers,
  type ModelArtifact,
} from "./desktop-targets";
import { acquireFile, RUNTIME_HOSTS, MAX_BYTES } from "./release-assets";

export const publishedModel: ModelArtifact = {
  name: MODEL_NAME,
  revision: MODEL_REVISION,
  archive: modelArtifactName(),
  sha256: "4a23a216615c6224b1dba9fcd067e2da0d0be2b460b39dbb468b185f152911c0",
  members: modelMembers(),
};
export async function acquirePublishedModel(
  directory: string,
  model = publishedModel,
  request = fetch,
) {
  if (
    model.archive !== publishedModel.archive ||
    model.sha256 !== publishedModel.sha256 ||
    model.name !== publishedModel.name ||
    model.revision !== publishedModel.revision ||
    Object.keys(model.members).length !==
      Object.keys(publishedModel.members).length ||
    Object.entries(publishedModel.members).some(
      ([name, hash]) => model.members[name] !== hash,
    )
  )
    throw new Error(
      "Common model differs from the pinned published archive; publish and pin the new model first",
    );
  await acquireFile(
    join(directory, model.archive),
    model.sha256,
    `https://github.com/InkyQuill/hieronymus/releases/download/v0.13.4/${model.archive}`,
    RUNTIME_HOSTS,
    MAX_BYTES,
    AbortSignal.timeout(300_000),
    request,
  );
}
if (import.meta.main) {
  const directory = Bun.argv[2] ?? "target/common-model";
  await acquirePublishedModel(directory);
  writeFileSync(
    join(directory, "common-model.json"),
    JSON.stringify(publishedModel, null, 2) + "\n",
  );
}
