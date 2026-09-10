#!/bin/sh
# Delegate ownership validation and data preservation to the installed native CLI.
set -eu
exec hiero uninstall "$@"
