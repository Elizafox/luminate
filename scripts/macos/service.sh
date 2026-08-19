#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "${repo_root}"

if ! sudo -n true; then
  printf '%s\n' 'error: the privileged macOS smoke phase requires non-interactive sudo' >&2
  exit 1
fi

printf '\n==> Privileged macOS service lifecycle\n'
START_SERVICE=1 packaging/install-macos.sh
sudo -n /usr/local/bin/luminated service status
sudo -n /usr/local/bin/luminated service stop
sudo -n /usr/local/bin/luminated service uninstall
