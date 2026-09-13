#!/bin/bash
set -euo pipefail
# Installer.app runs package scripts as root. Install the local server as the
# signed-in author, with their home directory and GUI bootstrap namespace.
[ "${3:-/}" = / ] || { echo 'Install Hieronymus on the current startup disk.' >&2; exit 1; }
author="$(/usr/bin/stat -f '%Su' /dev/console)"
case "$author" in ''|root|loginwindow|_*) echo 'Sign in to your Mac, then open the installer again.' >&2; exit 1;; esac
author_uid="$(/usr/bin/id -u "$author")"
[ "$author_uid" -ge 501 ] || { echo 'A signed-in user account is required.' >&2; exit 1; }
script_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
stage="$(/usr/bin/mktemp -d /private/tmp/hieronymus-setup.XXXXXXXX)"
trap '/bin/rm -rf "$stage"' EXIT
/bin/cp "$script_dir/install-hieronymus.sh" "$stage/install.sh"
/bin/chmod 500 "$stage/install.sh"
/usr/sbin/chown -R "$author_uid" "$stage"
/bin/launchctl asuser "$author_uid" /usr/bin/sudo -H -u "$author" /bin/bash "$stage/install.sh"
