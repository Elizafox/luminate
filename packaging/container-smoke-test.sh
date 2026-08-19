#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

# Smoke-test the packaged container image with Podman.
#
# This validates the packaged image in the same practical shape an operator
# would use: start the daemon container detached, then exec the installed CLI
# inside that same container. Runtime/state directories are bind-mounted from a
# temporary host directory so the test also covers the container entrypoint's
# root setup + privilege-drop path.

CONTAINER_ENGINE="${CONTAINER_ENGINE:-podman}"
IMAGE_TAG="${IMAGE_TAG:-luminate-packaging-test}"
CONTAINER_NAME="${CONTAINER_NAME:-luminate-smoke-$$}"
KEEP_CONTAINER="${KEEP_CONTAINER:-0}"
BUILD_IMAGE="${BUILD_IMAGE:-1}"

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temp_root=$(mktemp -d "${TMPDIR:-/tmp}/luminate-container-smoke.XXXXXX")
runtime_dir="${temp_root}/run"
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
  if [ "${KEEP_CONTAINER}" != "1" ]; then
    if [ -n "${container_id}" ]; then
      "${CONTAINER_ENGINE}" rm -f "${container_id}" >/dev/null 2>&1 || true
    fi
    remove_temp_root
  else
    printf 'kept container %s and temp root %s\n' "${CONTAINER_NAME}" "${temp_root}" >&2
  fi
}
trap cleanup EXIT INT TERM

mkdir -p "${runtime_dir}" "${state_dir}"

if [ "${BUILD_IMAGE}" = "1" ]; then
  "${CONTAINER_ENGINE}" build \
    -f "${repo_root}/packaging/Dockerfile" \
    -t "${IMAGE_TAG}" \
    "${repo_root}"
fi

# The demo plugin is kept off the default autoload path, so point the daemon at
# the shipped example config, which loads it from the examples directory.
container_id=$("${CONTAINER_ENGINE}" run \
  --detach \
  --name "${CONTAINER_NAME}" \
  -e LUMINATED_CONFIG=/usr/share/luminate/examples/luminated-demo.toml \
  -v "${runtime_dir}:/run/luminated:Z" \
  -v "${state_dir}:/var/lib/luminated:Z" \
  "${IMAGE_TAG}")

if [ -z "${container_id}" ]; then
  printf '%s\n' "container engine returned no container ID" >&2
  exit 1
fi

deadline=$(($(date +%s) + 30))
while ! "${CONTAINER_ENGINE}" exec "${CONTAINER_NAME}" /usr/bin/luminatectl ping >/dev/null 2>&1; do
  if ! "${CONTAINER_ENGINE}" inspect "${CONTAINER_NAME}" >/dev/null 2>&1; then
    printf '%s\n' "container disappeared before daemon became ready" >&2
    "${CONTAINER_ENGINE}" logs "${CONTAINER_NAME}" >&2 || true
    exit 1
  fi

  if [ "$(date +%s)" -ge "${deadline}" ]; then
    printf '%s\n' "timed out waiting for containerized daemon" >&2
    printf '%s\n' "--- container logs ---" >&2
    "${CONTAINER_ENGINE}" logs "${CONTAINER_NAME}" >&2 || true
    exit 1
  fi

  sleep 0.1
done

list_json=$("${CONTAINER_ENGINE}" exec "${CONTAINER_NAME}" /usr/bin/luminatectl list --json)
printf '%s\n' "${list_json}" | grep -F '"id": "demo-keyboard"' >/dev/null
printf '%s\n' "${list_json}" | grep -F '"id": "demo-case-lights"' >/dev/null

mutation_output=$(
  "${CONTAINER_ENGINE}" exec "${CONTAINER_NAME}" \
    /usr/bin/luminatectl set-effect --effect static \
      --device demo-keyboard \
      --surface zones \
      --element g1 \
      --rgb 'rgb(12, 34, 56)'
)

if [ "${mutation_output}" != "ok" ]; then
  printf 'unexpected mutation output: %s\n' "${mutation_output}" >&2
  exit 1
fi

printf '%s\n' "container packaging smoke test passed"
