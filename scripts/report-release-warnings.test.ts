import { expect, test } from "bun:test";
import {
  mkdtempSync,
  writeFileSync,
  readFileSync,
  existsSync,
  rmSync,
} from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

for (const mode of [
  "passed",
  "failed",
  "existing",
  "job-failed",
  "list-error",
  "create-error",
])
  test.skipIf(process.platform === "win32")(
    `release warning reporter: ${mode}`,
    () => {
      const root = mkdtempSync(join(tmpdir(), "hiero-warning-"));
      try {
        writeFileSync(
          join(root, "gh"),
          `#!/bin/sh
if [ "$2" = list ]; then
  if [ "$REPORT_MODE" = list-error ]; then exit 1; fi
  if [ "$REPORT_MODE" = existing ]; then echo 'Release warning: candidate / native / Linux'; fi
elif [ "$2" = create ]; then
  if [ "$REPORT_MODE" = create-error ]; then exit 1; fi
  printf '%s\\n' "$@" > "$REPORT_CALLS"
else exit 2
fi
`,
          { mode: 0o700 },
        );
        const calls = join(root, "calls");
        const result = Bun.spawnSync(
          ["bash", join(import.meta.dir, "report-release-warnings.sh")],
          {
            env: {
              ...process.env,
              PATH: `${root}:${process.env.PATH}`,
              RUNNER_TEMP: root,
              GITHUB_WORKFLOW: "candidate",
              GITHUB_JOB: "native",
              RELEASE_PLATFORM: "Linux",
              GITHUB_REPOSITORY: "owner/repo",
              GITHUB_RUN_ID: "123",
              REPORT_MODE: mode,
              REPORT_CALLS: calls,
              JOB_STATUS: mode === "job-failed" ? "failure" : "success",
              CHECK_RESULTS: JSON.stringify({
                smoke: {
                  outcome:
                    mode === "passed" || mode === "job-failed"
                      ? "success"
                      : "failure",
                },
              }),
            },
          },
        );
        expect(result.exitCode).toBe(0);
        if (mode.endsWith("-error")) {
          expect(result.stdout.toString()).toContain("::warning::Could not");
        }
        expect(existsSync(calls)).toBe(
          mode === "failed" || mode === "job-failed",
        );
        if (existsSync(calls)) {
          expect(
            readFileSync(join(root, "release-warning.md"), "utf8"),
          ).toContain("https://github.com/owner/repo/actions/runs/123");
          expect(readFileSync(calls, "utf8")).toContain("--body-file");
        }
      } finally {
        rmSync(root, { recursive: true, force: true });
      }
    },
  );
