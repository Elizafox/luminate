#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "${repo_root}"

printf '\n==> Formatting\n'
cargo fmt --all -- --check

printf '\n==> Clippy\n'
cargo clippy --workspace --all-targets --all-features -- -D warnings

printf '\n==> Workspace tests\n'
cargo test --workspace --all-features
