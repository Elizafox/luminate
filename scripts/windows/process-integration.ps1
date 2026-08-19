# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location -LiteralPath $RepoRoot

function Confirm-NativeSuccess {
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

Write-Host "`n==> CLI process integration tests"
cargo test -p luminate-cli --test cli_integration -- --ignored --test-threads=1
Confirm-NativeSuccess

Write-Host "`n==> Plugin conformance"
cargo test -p luminated --test plugin_conformance -- --ignored --test-threads=1
Confirm-NativeSuccess

Write-Host "`n==> Shared-memory process integration tests"
cargo test -p luminated --test shm_client_stream -- --ignored --test-threads=1
Confirm-NativeSuccess
