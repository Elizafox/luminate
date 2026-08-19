# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location -LiteralPath $RepoRoot

if (-not (Get-Command dbus-run-session.exe -ErrorAction SilentlyContinue)) {
    throw 'required command is unavailable: dbus-run-session.exe'
}

Write-Host "`n==> D-Bus process integration tests"
dbus-run-session.exe -- cargo test -p luminate-dbus --features dbus --test dbus_integration -- --ignored --test-threads=1
exit $LASTEXITCODE
