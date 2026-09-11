#!/usr/bin/env bash
# Desktop bootstrap uses the verified installer and its existing path overrides.
set -euo pipefail
exec "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/install.sh" --desktop "$@"
