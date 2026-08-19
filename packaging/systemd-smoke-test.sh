#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

# Boot the packaged systemd unit as PID 1 and exercise a complete service
# lifecycle, including persistence across a systemctl-managed restart.

CONTAINER_ENGINE="${CONTAINER_ENGINE:-podman}"
IMAGE_TAG="${IMAGE_TAG:-luminate-systemd-packaging-test}"
CONTAINER_NAME="${CONTAINER_NAME:-luminate-systemd-smoke-$$}"
KEEP_CONTAINER="${KEEP_CONTAINER:-0}"
BUILD_IMAGE="${BUILD_IMAGE:-1}"

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temp_root=$(mktemp -d "${TMPDIR:-/tmp}/luminate-systemd-smoke.XXXXXX")
state_dir="${temp_root}/state"
container_id=

remove_temp_root() {
  if rm -rf "${temp_root}" 2>/dev/null; then
    return
  fi
  if [ "$(basename -- "${CONTAINER_ENGINE}")" = "podman" ]; then
    "${CONTAINER_ENGINE}" unshare rm -rf "${temp_root}" 2>/dev/null || true
  fi
}

cleanup() {
  if [ "${KEEP_CONTAINER}" = "1" ]; then
    printf 'kept container %s and temp root %s\n' "${CONTAINER_NAME}" "${temp_root}" >&2
    return
  fi
  if [ -n "${container_id}" ]; then
    "${CONTAINER_ENGINE}" rm -f "${container_id}" >/dev/null 2>&1 || true
  fi
  remove_temp_root
}
trap cleanup EXIT INT TERM

mkdir -p "${state_dir}"

if [ "${BUILD_IMAGE}" = "1" ]; then
  "${CONTAINER_ENGINE}" build \
    -f "${repo_root}/packaging/systemd/Dockerfile" \
    -t "${IMAGE_TAG}" \
    "${repo_root}"
fi

container_id=$("${CONTAINER_ENGINE}" run \
  --detach \
  --name "${CONTAINER_NAME}" \
  --privileged \
  --systemd=always \
  -v "${state_dir}:/var/lib/luminated:Z" \
  "${IMAGE_TAG}")

if [ -z "${container_id}" ]; then
  printf '%s\n' "container engine returned no container ID" >&2
  exit 1
fi

container_exec() {
  "${CONTAINER_ENGINE}" exec "${CONTAINER_NAME}" "$@"
}

dump_diagnostics() {
  container_exec systemctl status luminated.service >&2 || true
  container_exec journalctl -u luminated.service --no-pager >&2 || true
  "${CONTAINER_ENGINE}" logs "${CONTAINER_NAME}" >&2 || true
}

wait_for_cli() {
  deadline=$(($(date +%s) + 45))
  while ! container_exec /usr/bin/luminatectl ping >/dev/null 2>&1; do
    if [ "$(date +%s)" -ge "${deadline}" ]; then
      printf '%s\n' "timed out waiting for systemd-managed luminated" >&2
      dump_diagnostics
      exit 1
    fi
    sleep 0.1
  done
}

wait_for_cli
container_exec systemctl is-active --quiet luminated.service
test "$(container_exec systemctl show -p User --value luminated.service)" = "luminated"
test "$(container_exec systemctl show -p Group --value luminated.service)" = "luminate"

list_json=$(container_exec /usr/bin/luminatectl list --json)
printf '%s\n' "${list_json}" | grep -F '"id": "demo-keyboard"' >/dev/null

mutation_output=$(container_exec /usr/bin/luminatectl set-effect --effect static \
  --device demo-keyboard \
  --surface zones \
  --element g1 \
  --rgb 'rgb(12, 34, 56)')
test "${mutation_output}" = "ok"
container_exec test -s /var/lib/luminated/state.json

container_exec systemctl restart luminated.service
wait_for_cli

state_json=$(container_exec /usr/bin/luminatectl state --device demo-keyboard)
printf '%s\n' "${state_json}" | grep -F '"value": 12' >/dev/null
printf '%s\n' "${state_json}" | grep -F '"value": 34' >/dev/null
printf '%s\n' "${state_json}" | grep -F '"value": 56' >/dev/null

container_exec systemctl stop luminated.service
if container_exec test -S /run/luminated/luminated.sock; then
  printf '%s\n' "systemd stop left the daemon socket behind" >&2
  exit 1
fi

printf '%s\n' "systemd packaging smoke test passed"
