#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(dirname -- "${script_dir}")
temp_root=$(mktemp -d "${TMPDIR:-/tmp}/luminate-major-check.XXXXXX")

cleanup() {
  rm -rf -- "${temp_root}"
}
trap cleanup EXIT HUP INT TERM

step() {
  printf '\n==> %s\n' "$1"
}

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'required command is unavailable: %s\n' "$1" >&2
    exit 1
  fi
}

cd "${repo_root}"

for command_name in \
  cargo cargo-audit cargo-deb cargo-generate-rpm cbindgen cpio dbus-run-session \
  git podman readelf rpm rpm2cpio rustc; do
  require "${command_name}"
done

step "Formatting"
cargo fmt --all -- --check

step "Clippy"
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

step "Workspace tests"
cargo test --locked --workspace --all-features

step "C API integration tests"
cargo test --locked -p libluminate --test c_api_smoke

step "CLI process integration tests"
cargo test --locked -p luminate-cli --test cli_integration -- --ignored --test-threads=1

step "D-Bus process integration tests"
dbus-run-session -- \
  cargo test --locked -p luminate-dbus --features dbus --test dbus_integration -- --ignored --test-threads=1

step "Plugin conformance"
cargo test --locked -p luminated --test plugin_conformance -- --ignored

step "Dependency audit"
cargo audit

step "Generated C header"
cbindgen \
  --config crates/libluminate/cbindgen.toml \
  --crate libluminate \
  --output "${temp_root}/luminate.h"
cmp crates/libluminate/luminate.h "${temp_root}/luminate.h"

step "Host packaged-layout smoke test, including D-Bus"
SMOKE_DBUS=1 packaging/smoke-test.sh

step "DEB artefact build"
packaging/build-deb.sh

step "RPM artefact and systemd lifecycle smoke test"
packaging/rpm-smoke-test.sh

step "Container image smoke test"
packaging/container-smoke-test.sh

step "Systemd image lifecycle smoke test"
packaging/systemd-smoke-test.sh

step "Alpine APK, musl, and OpenRC smoke test"
packaging/alpine-smoke-test.sh

step "Container smoke cleanup regression tests"
packaging/tests/container-smoke-cleanup.sh

printf '\nAll major-change checks passed.\n'
