#!/bin/sh
# Run from a checkout; the native installer requires explicit release inputs.
set -eu
exec bash "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/scripts/install.sh" "$@"
