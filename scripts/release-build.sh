#!/usr/bin/env bash
# Compatibility entry point; the exact-target v2 producer owns packaging.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
exec bun scripts/release-build.ts "$@"
