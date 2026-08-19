# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

param(
    [switch]$Privileged,
    [switch]$RequireDBus,
    [switch]$SkipDBus
)

$ErrorActionPreference = 'Stop'

if ($RequireDBus -and $SkipDBus) {
    throw '-RequireDBus and -SkipDBus are mutually exclusive'
}

function Invoke-Phase {
    param([Parameter(Mandatory)][string]$Path)

    & $Path
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

Invoke-Phase (Join-Path $PSScriptRoot 'workspace.ps1')
Invoke-Phase (Join-Path $PSScriptRoot 'process-integration.ps1')

if ($SkipDBus) {
    Write-Host "`n==> D-Bus process integration tests (skipped by request)"
} elseif (Get-Command dbus-run-session.exe -ErrorAction SilentlyContinue) {
    & (Join-Path $PSScriptRoot 'dbus.ps1')
    if ($LASTEXITCODE -ne 0) {
        if ($RequireDBus) {
            exit $LASTEXITCODE
        }
        Write-Warning 'Tier-3 Windows D-Bus integration failed; continuing the smoke test.'
    }
} elseif ($RequireDBus) {
    throw 'required command is unavailable: dbus-run-session.exe'
} else {
    Write-Host "`n==> D-Bus process integration tests (skipped: dbus-run-session.exe is unavailable)"
}

if ($Privileged) {
    Invoke-Phase (Join-Path $PSScriptRoot 'service.ps1')
} else {
    Write-Host "`n==> Privileged Windows service lifecycle (skipped; pass -Privileged to enable)"
}

Write-Host "`nWindows smoke test passed."
