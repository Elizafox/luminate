# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

$ErrorActionPreference = 'Stop'

& (Join-Path $PSScriptRoot 'windows\smoke-test.ps1') @args
exit $LASTEXITCODE
