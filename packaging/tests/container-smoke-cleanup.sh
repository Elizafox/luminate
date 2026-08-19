#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/luminate-container-cleanup-test.XXXXXX")
fake_engine="${test_root}/fake-container-engine"

cleanup() {
  rm -rf "${test_root}"
}
trap cleanup EXIT INT TERM

cat >"${fake_engine}" <<'EOF'
#!/usr/bin/env sh
set -eu

printf '%s\n' "$*" >>"${FAKE_ENGINE_LOG}"

case "$1" in
  run)
    if [ "${FAKE_ENGINE_MODE}" = "collision" ]; then
      exit 125
    fi
    printf '%s\n' "owned-container-id"
    ;;
  exec)
    case "$*" in
      *" /usr/bin/luminatectl list --json")
        printf '%s\n' '[{"id": "demo-keyboard"}, {"id": "demo-case-lights"}]'
        ;;
      *" /usr/bin/luminatectl set-effect --effect static "*)
        printf '%s\n' "ok"
        ;;
    esac
    ;;
  rm)
    ;;
  *)
    printf 'unexpected fake container-engine command: %s\n' "$*" >&2
    exit 2
    ;;
esac
EOF
chmod +x "${fake_engine}"

collision_log="${test_root}/collision.log"
if FAKE_ENGINE_LOG="${collision_log}" \
  FAKE_ENGINE_MODE=collision \
  BUILD_IMAGE=0 \
  TMPDIR="${test_root}" \
  CONTAINER_ENGINE="${fake_engine}" \
  CONTAINER_NAME=pre-existing \
  "${repo_root}/packaging/container-smoke-test.sh" >/dev/null 2>&1; then
  printf '%s\n' "container smoke test unexpectedly accepted a name collision" >&2
  exit 1
fi
if grep -F "rm -f" "${collision_log}" >/dev/null; then
  printf '%s\n' "name-collision cleanup attempted to remove a container" >&2
  exit 1
fi

success_log="${test_root}/success.log"
FAKE_ENGINE_LOG="${success_log}" \
  FAKE_ENGINE_MODE=success \
  BUILD_IMAGE=0 \
  TMPDIR="${test_root}" \
  CONTAINER_ENGINE="${fake_engine}" \
  CONTAINER_NAME=smoke-test-name \
  "${repo_root}/packaging/container-smoke-test.sh" >/dev/null
if ! grep -Fx "rm -f owned-container-id" "${success_log}" >/dev/null; then
  printf '%s\n' "successful cleanup did not remove the returned container ID" >&2
  exit 1
fi
if grep -Fx "rm -f smoke-test-name" "${success_log}" >/dev/null; then
  printf '%s\n' "successful cleanup removed by name instead of owned ID" >&2
  exit 1
fi

keep_log="${test_root}/keep.log"
FAKE_ENGINE_LOG="${keep_log}" \
  FAKE_ENGINE_MODE=success \
  BUILD_IMAGE=0 \
  KEEP_CONTAINER=1 \
  TMPDIR="${test_root}" \
  CONTAINER_ENGINE="${fake_engine}" \
  CONTAINER_NAME=kept-smoke-test \
  "${repo_root}/packaging/container-smoke-test.sh" >/dev/null 2>&1
if grep -F "rm -f" "${keep_log}" >/dev/null; then
  printf '%s\n' "KEEP_CONTAINER=1 cleanup attempted to remove a container" >&2
  exit 1
fi

printf '%s\n' "container smoke cleanup tests passed"
