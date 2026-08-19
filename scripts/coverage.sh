#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

usage() {
  printf 'usage: %s [--workspace | --luminated | --libluminate]\n' "$0" >&2
}

if [ "$#" -gt 1 ]; then
  usage
  exit 2
fi

case "${1:---workspace}" in
--workspace)
  exec "${script_dir}/coverage/workspace.sh"
  ;;
--luminated)
  exec "${script_dir}/coverage/luminated.sh"
  ;;
--libluminate)
  exec "${script_dir}/coverage/libluminate.sh"
  ;;
--help)
  usage
  ;;
*)
  usage
  exit 2
  ;;
esac
