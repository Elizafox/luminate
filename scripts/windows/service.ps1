# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location -LiteralPath $RepoRoot

$Identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$Principal = [Security.Principal.WindowsPrincipal]::new($Identity)
if (-not $Principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'the privileged Windows smoke phase must run from an elevated session'
}

Write-Host "`n==> Privileged Windows service lifecycle"
cargo test -p luminated --test windows_service_install -- --ignored --test-threads=1
exit $LASTEXITCODE
