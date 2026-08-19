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

Write-Host "`n==> Formatting"
cargo fmt --all -- --check
Confirm-NativeSuccess

Write-Host "`n==> Clippy"
cargo clippy --workspace --all-targets --all-features -- -D warnings
Confirm-NativeSuccess

Write-Host "`n==> Workspace tests"
cargo test --workspace --all-features
Confirm-NativeSuccess
