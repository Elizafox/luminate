#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

if ! command -v dbus-run-session >/dev/null 2>&1; then
  printf '%s\n' 'required command is unavailable: dbus-run-session' >&2
  exit 1
fi

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "${repo_root}"

printf '\n==> D-Bus process integration tests\n'
dbus-run-session -- \
  cargo test -p luminate-dbus --features dbus --test dbus_integration -- --ignored --test-threads=1
