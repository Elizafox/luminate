#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

# Optional Windows libvirt guest integration test.
#
# Configuration:
#   WINDOWS_VM                 libvirt domain (default: Windows 11)
#   WINDOWS_HOST               guest address (default: 192.168.122.79)
#   WINDOWS_USER               SSH account (default: development)
#   WINDOWS_REMOTE_WORKSPACE   disposable guest path (default: luminate)
#   VM_BOOT_TIMEOUT            boot/SSH timeout in seconds (default: 300)
#   VM_SHUTDOWN_TIMEOUT        shutdown timeout in seconds (default: 180)

set -euo pipefail

vm_name=${WINDOWS_VM:-Windows 11}
vm_host=${WINDOWS_HOST:-192.168.122.79}
vm_user=${WINDOWS_USER:-development}
remote_workspace=${WINDOWS_REMOTE_WORKSPACE:-luminate}
boot_timeout=${VM_BOOT_TIMEOUT:-300}
shutdown_timeout=${VM_SHUTDOWN_TIMEOUT:-180}
libvirt_uri=${LIBVIRT_URI:-qemu:///system}
vm_started=0
shutdown_complete=0

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(dirname -- "${script_dir}")
archive=$(mktemp "${TMPDIR:-/tmp}/luminate-windows-smoke.XXXXXX.tar.gz")

log() {
  printf '\n==> %s\n' "$1"
}

domain_state() {
  virsh --connect "${libvirt_uri}" domstate "${vm_name}" 2>/dev/null | tr -d '\r'
}

wait_until() {
  description=$1
  timeout=$2
  shift 2
  deadline=$((SECONDS + timeout))
  until "$@"; do
    if (( SECONDS >= deadline )); then
      printf 'timed out waiting for %s\n' "${description}" >&2
      return 1
    fi
    sleep 2
  done
}

ping_guest() {
  ping -c 1 -W 1 "${vm_host}" >/dev/null 2>&1
}

ssh_guest() {
  ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 \
    -o ServerAliveInterval=5 -o ServerAliveCountMax=2 \
    "${vm_user}@${vm_host}" "$@"
}

ssh_ready() {
  ssh_guest 'powershell.exe -NoProfile -NonInteractive -Command "exit 0"' >/dev/null 2>&1
}

domain_is_off() {
  [ "$(domain_state)" = 'shut off' ]
}

graceful_shutdown() {
  if [ "${vm_started}" -eq 0 ] || [ "${shutdown_complete}" -eq 1 ]; then
    return
  fi

  log "Requesting a graceful Windows shutdown"
  ssh_guest 'powershell.exe -NoProfile -NonInteractive -Command "Stop-Computer -Force"' \
    >/dev/null 2>&1 ||
    virsh --connect "${libvirt_uri}" shutdown "${vm_name}" >/dev/null 2>&1 || true

  if wait_until "${vm_name} to shut off" "${shutdown_timeout}" domain_is_off; then
    shutdown_complete=1
  else
    printf '%s remains running; refusing to force it off\n' "${vm_name}" >&2
    return 1
  fi
}

cleanup() {
  status=$?
  trap - EXIT HUP INT TERM
  graceful_shutdown || status=1
  rm -f -- "${archive}"
  exit "${status}"
}
trap cleanup EXIT HUP INT TERM

for command_name in virsh ping ssh scp tar; do
  command -v "${command_name}" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "${command_name}" >&2
    exit 1
  }
done

if [ "$(domain_state)" != 'shut off' ]; then
  printf '%s is already running; refusing to take ownership of it\n' "${vm_name}" >&2
  exit 1
fi

log "Archiving the current workspace"
tar -C "${repo_root}" --exclude=.git --exclude=target -czf "${archive}" .

log "Starting ${vm_name}"
virsh --connect "${libvirt_uri}" start "${vm_name}"
vm_started=1
wait_until "${vm_host} to answer ping" "${boot_timeout}" ping_guest
wait_until "SSH on ${vm_host}" "${boot_timeout}" ssh_ready

log "Replacing the disposable remote workspace"
ssh_guest "powershell.exe -NoProfile -NonInteractive -Command \"Remove-Item -LiteralPath '${remote_workspace}' -Recurse -Force -ErrorAction SilentlyContinue; New-Item -ItemType Directory -Path '${remote_workspace}' -Force | Out-Null\""
scp -q -o BatchMode=yes -o StrictHostKeyChecking=accept-new "${archive}" \
  "${vm_user}@${vm_host}:${remote_workspace}/workspace.tar.gz"
ssh_guest "powershell.exe -NoProfile -NonInteractive -Command \"Set-Location -LiteralPath '${remote_workspace}'; tar -xzf workspace.tar.gz; Remove-Item workspace.tar.gz\""

log "Running the platform-local Windows smoke test"
ssh_guest "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command \"\
  \$ErrorActionPreference = 'Stop'; \
  Set-Location -LiteralPath '${remote_workspace}'; \
  \$env:CARGO_TARGET_DIR = Join-Path \$env:USERPROFILE '.cache\\luminate-windows-smoke-target'; \
  Remove-Item -LiteralPath 'target' -Recurse -Force -ErrorAction SilentlyContinue; \
  New-Item -ItemType Junction -Path 'target' -Target \$env:CARGO_TARGET_DIR | Out-Null; \
  & '.\\scripts\\smoke-test-windows.ps1' -Privileged; \
  exit \$LASTEXITCODE\
\""

graceful_shutdown
log "Windows smoke test passed"
