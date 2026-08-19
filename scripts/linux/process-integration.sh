#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "${repo_root}"

printf '\n==> CLI process integration tests\n'
cargo test -p luminate-cli --test cli_integration -- --ignored --test-threads=1

printf '\n==> Plugin conformance\n'
cargo test -p luminated --test plugin_conformance -- --ignored --test-threads=1

printf '\n==> Shared-memory process integration tests\n'
cargo test -p luminated --test shm_client_stream -- --ignored --test-threads=1

printf '\n==> HTTP LAN security process integration tests\n'
cargo build -p luminated -p luminate-http
cargo test -p luminate-http --test process_security -- --ignored --test-threads=1
