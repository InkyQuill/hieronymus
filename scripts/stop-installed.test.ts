import { afterEach, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const roots: string[] = [];
const unix = test.skipIf(process.platform === "win32");
const busy =
  "hiero: another lifecycle operation is in progress for this data root; retry after it finishes";
afterEach(() => {
  for (const root of roots.splice(0))
    rmSync(root, { recursive: true, force: true });
});

function exercise(message: string, status: number) {
  const root = mkdtempSync(join(tmpdir(), "hiero stop cleanup "));
  roots.push(root);
  const cli = join(root, "hiero");
  const calls = join(root, "calls");
  writeFileSync(
    cli,
    `#!/usr/bin/env bash
set -eu
[ "$1" = stop ] && [ "$2" = --data-root ] && [ "$3" = "$STOP_FIXTURE_DATA" ]
if [ ! -f "$STOP_FIXTURE_CALLS" ]; then
  printf '1' > "$STOP_FIXTURE_CALLS"
  printf '%s\\n' "$STOP_FIXTURE_MESSAGE" >&2
  exit "$STOP_FIXTURE_STATUS"
fi
printf '2' > "$STOP_FIXTURE_CALLS"
printf 'stopped\\n'
`,
    { mode: 0o700 },
  );
  const result = Bun.spawnSync(
    ["bash", join(import.meta.dir, "stop-installed.sh"), cli, join(root, "data"), join(root, "stop.log")],
    {
      env: {
        ...process.env,
        STOP_FIXTURE_CALLS: calls,
        STOP_FIXTURE_MESSAGE: message,
        STOP_FIXTURE_STATUS: String(status),
        STOP_FIXTURE_DATA: join(root, "data"),
      },
    },
  );
  return { ...result, calls: readFileSync(calls, "utf8") };
}

unix("cleanup retries the exact pre-action busy refusal", () => {
  const result = exercise(busy, 2);
  expect(result.exitCode).toBe(0);
  expect(result.calls).toBe("2");
  expect(result.stdout.toString()).toContain("stopped");
});

for (const [name, message, status] of [
  ["foreign registration", "hiero: Foreign daemon login definition", 2],
  ["additional diagnostic", `${busy}\nhiero: another error`, 2],
  ["different exit code", busy, 7],
] as const) {
  unix(`cleanup preserves ${name} without retry`, () => {
    const result = exercise(message, status);
    expect(result.exitCode).toBe(status);
    expect(result.calls).toBe("1");
    expect(result.stderr.toString().trimEnd()).toBe(message);
  });
}
