#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

# Render a packaging-root tree using packaging/install-linux.sh's own path
# defaults, for cargo-deb/cargo-generate-rpm to package as pre-built assets.
#
# This intentionally reuses install-linux.sh rather than re-implementing template
# rendering in Cargo.toml metadata, so the deb/rpm outputs always match the
# same paths/permissions as the host-installed and container-installed
# layouts.

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
stage_dir="${repo_root}/target/pkgroot"

rm -rf "${stage_dir}"

DESTDIR="${stage_dir}" \
BUILD_PROFILE="${BUILD_PROFILE:-release}" \
"${repo_root}/packaging/install-linux.sh"

printf 'staged packaging root at %s\n' "${stage_dir}"
