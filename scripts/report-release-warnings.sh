#!/usr/bin/env bash
# Best-effort diagnostics. Call even after failed steps with CHECK_RESULTS=toJSON(steps).
set -euo pipefail
if [ "${JOB_STATUS:-success}" != failure ] && ! printf '%s' "$CHECK_RESULTS" | grep -Eq '"outcome"[[:space:]]*:[[:space:]]*"failure"'; then exit 0; fi
echo '::warning::Release checks need attention; recording a non-blocking issue.'
title="Release warning: $GITHUB_WORKFLOW / $GITHUB_JOB / ${RELEASE_PLATFORM:-all}"
if ! existing=$(gh issue list --repo "$GITHUB_REPOSITORY" --state open --search "$title in:title" --json title --jq '.[].title'); then
  echo '::warning::Could not query existing release-warning issues.'
  exit 0
fi
if ! printf '%s\n' "$existing" | grep -Fxq "$title"; then
  printf 'Non-blocking release warning. Only P0 issues block publication.\n\nRun: https://github.com/%s/actions/runs/%s\n\nStep results:\n```json\n%s\n```\n' "$GITHUB_REPOSITORY" "$GITHUB_RUN_ID" "$CHECK_RESULTS" > "$RUNNER_TEMP/release-warning.md"
  gh issue create --repo "$GITHUB_REPOSITORY" --title "$title" --body-file "$RUNNER_TEMP/release-warning.md" ||
    echo '::warning::Could not create the release-warning issue.'
fi
