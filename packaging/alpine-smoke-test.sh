#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

# Build Luminate natively with musl, install its OpenRC integration, and run an
# end-to-end lifecycle test in Alpine. This intentionally does not use the
# Debian container entrypoint or any systemd compatibility layer.

CONTAINER_ENGINE="${CONTAINER_ENGINE:-podman}"
IMAGE_TAG="${IMAGE_TAG:-luminate-alpine-packaging-test}"
CONTAINER_NAME="${CONTAINER_NAME:-luminate-alpine-smoke-$$}"
KEEP_CONTAINER="${KEEP_CONTAINER:-0}"
BUILD_IMAGE="${BUILD_IMAGE:-1}"

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
container_id=

cleanup() {
  if [ "${KEEP_CONTAINER}" != "1" ] && [ -n "${container_id}" ]; then
    "${CONTAINER_ENGINE}" rm -f "${container_id}" >/dev/null 2>&1 || true
  elif [ "${KEEP_CONTAINER}" = "1" ]; then
    printf 'kept container %s\n' "${CONTAINER_NAME}" >&2
  fi
}
trap cleanup EXIT INT TERM

if [ "${BUILD_IMAGE}" = "1" ]; then
  "${CONTAINER_ENGINE}" build \
    -f "${repo_root}/packaging/alpine/Dockerfile" \
    -t "${IMAGE_TAG}" \
    "${repo_root}"
fi

container_id=$("${CONTAINER_ENGINE}" run \
  --detach \
  --name "${CONTAINER_NAME}" \
  "${IMAGE_TAG}" \
  /bin/sh -c 'trap "exit 0" TERM INT; while :; do sleep 3600 & wait $!; done')

if [ -z "${container_id}" ]; then
  printf '%s\n' "container engine returned no container ID" >&2
  exit 1
fi

container_exec() {
  "${CONTAINER_ENGINE}" exec "${CONTAINER_NAME}" "$@"
}

dump_diagnostics() {
  container_exec rc-service luminated status >&2 || true
  container_exec ps -ef >&2 || true
  "${CONTAINER_ENGINE}" logs "${CONTAINER_NAME}" >&2 || true
}

wait_for_cli() {
  deadline=$(($(date +%s) + 30))
  while ! container_exec /usr/bin/luminatectl ping >/dev/null 2>&1; do
    if [ "$(date +%s)" -ge "${deadline}" ]; then
      printf '%s\n' "timed out waiting for OpenRC-managed luminated" >&2
      dump_diagnostics
      exit 1
    fi
    sleep 0.1
  done
}

# A normal container does not boot a full init process, so establish OpenRC's
# runlevel marker explicitly, then use rc-service for the daemon lifecycle.
container_exec mkdir -p /run/openrc
container_exec touch /run/openrc/softlevel
container_exec /bin/sh -c \
  'printf "%s\n" "luminated_config=/usr/share/luminate/examples/luminated-demo.toml" > /etc/conf.d/luminated'

# Verify the installed shared objects resolve under musl before loading a
# native plugin through the daemon.
container_exec ldd /usr/bin/luminated >/dev/null
container_exec ldd /usr/lib/libluminate.so >/dev/null
container_exec ldd \
  /usr/lib/luminate/plugins/libluminate_plugin_lifx.so >/dev/null
container_exec ldd \
  /usr/lib/luminate/plugins/libluminate_plugin_philips_hue.so >/dev/null
container_exec ldd \
  /usr/lib/luminate/plugins/libluminate_plugin_linux_leds.so >/dev/null
container_exec ldd \
  /usr/lib/luminate/plugins/libluminate_plugin_wled.so >/dev/null
container_exec ldd \
  /usr/lib/luminate/plugins/libluminate_plugin_razer.so >/dev/null
container_exec ldd \
  /usr/lib/luminate/plugins-examples/libluminate_plugin_demo_system.so >/dev/null

container_exec rc-service luminated start
wait_for_cli

list_json=$(container_exec /usr/bin/luminatectl list --json)
printf '%s\n' "${list_json}" | grep -F '"id": "demo-keyboard"' >/dev/null
printf '%s\n' "${list_json}" | grep -F '"id": "demo-case-lights"' >/dev/null

mutation_output=$(container_exec /usr/bin/luminatectl set-effect --effect static \
  --device demo-keyboard \
  --surface zones \
  --element g1 \
  --rgb 'rgb(12, 34, 56)')
if [ "${mutation_output}" != "ok" ]; then
  printf 'unexpected mutation output: %s\n' "${mutation_output}" >&2
  exit 1
fi
container_exec test -s /var/lib/luminated/state.json

container_exec rc-service luminated stop
if container_exec test -S /run/luminated/luminated.sock; then
  printf '%s\n' "OpenRC stop left the daemon socket behind" >&2
  exit 1
fi

container_exec rc-service luminated start
wait_for_cli

state_json=$(container_exec /usr/bin/luminatectl state --device demo-keyboard)
printf '%s\n' "${state_json}" | grep -F '"value": 12' >/dev/null
printf '%s\n' "${state_json}" | grep -F '"value": 34' >/dev/null
printf '%s\n' "${state_json}" | grep -F '"value": 56' >/dev/null

container_exec rc-service luminated stop
printf '%s\n' "Alpine/musl/OpenRC packaging smoke test passed"
