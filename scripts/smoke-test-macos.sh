#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
exec "${script_dir}/macos/smoke-test.sh" "$@"
