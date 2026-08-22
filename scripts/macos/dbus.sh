#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

if ! command -v dbus-run-session >/dev/null 2>&1; then
  printf '%s\n' 'required command is unavailable: dbus-run-session' >&2
  exit 1
fi

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "${script_dir}/../.." && pwd)
cd "${repo_root}"

printf '\n==> D-Bus process integration tests\n'
LUMINATE_DBUS_TEST_BUS=1 dbus-run-session --dbus-daemon="${script_dir}/private-dbus-daemon.sh" -- \
  cargo test -p luminate-dbus --features dbus --test dbus_integration -- --ignored --test-threads=1
