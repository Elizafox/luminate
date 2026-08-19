#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

if [ "$(uname -s)" != Darwin ]; then
  printf '%s\n' 'error: the macOS smoke test must run on macOS' >&2
  exit 1
fi
if ! command -v brew >/dev/null 2>&1; then
  printf '%s\n' 'required command is unavailable: brew' >&2
  exit 1
fi

llvm_prefix=$(brew --prefix llvm)
export CC="${llvm_prefix}/bin/clang"
export CXX="${llvm_prefix}/bin/clang++"
if [ ! -x "${CC}" ] || [ ! -x "${CXX}" ]; then
  printf '%s\n' 'Homebrew LLVM clang and clang++ are required' >&2
  exit 1
fi

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "${repo_root}"

printf '\n==> Formatting\n'
cargo fmt --all -- --check

printf '\n==> Clippy\n'
cargo clippy --workspace --all-targets --all-features -- -D warnings

printf '\n==> Workspace tests\n'
cargo test --workspace --all-features
