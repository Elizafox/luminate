#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

# Optional macOS libvirt guest integration test.
#
# Configuration:
#   MACOS_VM                 libvirt domain (default: macOS)
#   MACOS_HOST               guest address (default: 192.168.122.134)
#   MACOS_USER               SSH account (default: elizabeth)
#   MACOS_REMOTE_WORKSPACE   disposable guest path (default: luminate)
#   VM_BOOT_TIMEOUT          boot/SSH timeout in seconds (default: 600)
#   VM_SHUTDOWN_TIMEOUT      shutdown timeout in seconds (default: 180)

set -euo pipefail

vm_name=${MACOS_VM:-macOS}
vm_host=${MACOS_HOST:-192.168.122.134}
vm_user=${MACOS_USER:-elizabeth}
remote_workspace=${MACOS_REMOTE_WORKSPACE:-luminate}
boot_timeout=${VM_BOOT_TIMEOUT:-600}
shutdown_timeout=${VM_SHUTDOWN_TIMEOUT:-180}
libvirt_uri=${LIBVIRT_URI:-qemu:///system}
vm_started=0
shutdown_complete=0

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(dirname -- "${script_dir}")
archive=$(mktemp "${TMPDIR:-/tmp}/luminate-macos-smoke.XXXXXX.tar.gz")

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
  ssh_guest 'true' >/dev/null 2>&1
}

ssh_port_open() {
  ssh-keyscan -T 2 "${vm_host}" >/dev/null 2>&1
}

domain_is_off() {
  [ "$(domain_state)" = 'shut off' ]
}

graceful_shutdown() {
  if [ "${vm_started}" -eq 0 ] || [ "${shutdown_complete}" -eq 1 ]; then
    return
  fi

  log "Requesting a graceful macOS shutdown"
  ssh_guest "sudo -n shutdown -h now >/dev/null 2>&1 &" >/dev/null 2>&1 ||
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

for command_name in virsh ping ssh ssh-keyscan scp tar; do
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
wait_until "SSH port on ${vm_host}" "${boot_timeout}" ssh_port_open
wait_until "SSH on ${vm_host}" "${boot_timeout}" ssh_ready

log "Replacing the disposable remote workspace"
ssh_guest "rm -rf -- '${remote_workspace}' && mkdir -p -- '${remote_workspace}'"
scp -q -o BatchMode=yes -o StrictHostKeyChecking=accept-new "${archive}" \
  "${vm_user}@${vm_host}:${remote_workspace}/workspace.tar.gz"
ssh_guest "cd '${remote_workspace}' && tar -xzf workspace.tar.gz && rm workspace.tar.gz"

log "Running the platform-local macOS smoke test"
ssh_guest "cd '${remote_workspace}' && bash -lc '
  set -euo pipefail
  export CARGO_TARGET_DIR=\"\${HOME}/.cache/luminate-macos-smoke-target\"
  rm -rf target
  ln -s \"\${CARGO_TARGET_DIR}\" target
  scripts/smoke-test-macos.sh --privileged
'"

graceful_shutdown
log "macOS smoke test passed"
