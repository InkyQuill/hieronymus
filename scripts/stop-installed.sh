#!/usr/bin/env bash
# CI cleanup may overlap the installer's final desktop operation.
set -euo pipefail
cli="${1:?installed CLI required}"
data="${2:?data root required}"
output="${3:?output log required}"
stop_deadline=$((SECONDS + 30))
busy='hiero: another lifecycle operation is in progress for this data root; retry after it finishes'
while :; do
  if "$cli" stop --data-root "$data" > "$output" 2>&1; then
    cat "$output"
    exit 0
  else
    status=$?
  fi
  # This refusal occurs before stop mutates anything. Never retry other errors.
  if [ "$status" -ne 2 ] || [ "$(cat "$output")" != "$busy" ] || [ "$SECONDS" -ge "$stop_deadline" ]; then
    cat "$output" >&2
    exit "$status"
  fi
  printf 'Waiting for the current lifecycle operation before CI cleanup…\n'
  sleep 1
done
