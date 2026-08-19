#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

privileged=0
dbus_mode=auto

usage() {
  printf 'usage: %s [--privileged] [--require-dbus | --skip-dbus]\n' "$0" >&2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
  --privileged) privileged=1 ;;
  --require-dbus)
    if [ "${dbus_mode}" = skip ]; then
      usage
      exit 2
    fi
    dbus_mode=required
    ;;
  --skip-dbus)
    if [ "${dbus_mode}" = required ]; then
      usage
      exit 2
    fi
    dbus_mode=skip
    ;;
  *)
    usage
    exit 2
    ;;
  esac
  shift
done

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

"${script_dir}/workspace.sh"
"${script_dir}/process-integration.sh"

if [ "${dbus_mode}" = skip ]; then
  printf '\n==> D-Bus process integration tests (skipped by request)\n'
elif command -v dbus-run-session >/dev/null 2>&1; then
  if "${script_dir}/dbus.sh"; then
    :
  elif [ "${dbus_mode}" = required ]; then
    exit 1
  else
    printf '%s\n' \
      'warning: tier-2 macOS D-Bus integration failed; continuing the smoke test' >&2
  fi
elif [ "${dbus_mode}" = required ]; then
  printf '%s\n' 'required command is unavailable: dbus-run-session' >&2
  exit 1
else
  printf '\n==> D-Bus process integration tests (skipped: dbus-run-session is unavailable)\n'
fi

if [ "${privileged}" -eq 1 ]; then
  "${script_dir}/service.sh"
else
  printf '\n==> Privileged macOS service lifecycle (skipped; pass --privileged to enable)\n'
fi

printf '\nmacOS smoke test passed.\n'
