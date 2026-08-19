#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

# Build a .deb using cargo-deb, from packaging/build-stage.sh's rendered
# tree. Requires cargo-deb (cargo install cargo-deb).

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
INSTALL_DBUS="${INSTALL_DBUS:-0}"
INSTALL_DBUS_POLKIT="${INSTALL_DBUS_POLKIT:-0}"

"${repo_root}/packaging/build-stage.sh"
cd "${repo_root}"

set -- cargo deb --no-build --no-strip -p luminated
if [ "${INSTALL_DBUS_POLKIT}" = "1" ]; then
  set -- "$@" --variant dbus-polkit
elif [ "${INSTALL_DBUS}" = "1" ]; then
  set -- "$@" --variant dbus
fi
"$@"
