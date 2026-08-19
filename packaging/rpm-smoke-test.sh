#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

# Build the real .rpm artifact, install it inside a Fedora container via
# dnf (exercising the post_install/pre_uninstall scriptlets and dependency
# resolution, not just the staged tree), and run it as PID 1 under systemd
# to confirm the packaged unit starts, controls the demo device, and
# survives a systemctl-managed restart.

CONTAINER_ENGINE="${CONTAINER_ENGINE:-podman}"
IMAGE_TAG="${IMAGE_TAG:-luminate-rpm-packaging-test}"
CONTAINER_NAME="${CONTAINER_NAME:-luminate-rpm-smoke-$$}"
KEEP_CONTAINER="${KEEP_CONTAINER:-0}"
BUILD_IMAGE="${BUILD_IMAGE:-1}"
BUILD_RPM="${BUILD_RPM:-1}"
VERIFY_RPM_ONLY="${VERIFY_RPM_ONLY:-0}"
BUILD_TARGET="${BUILD_TARGET:-}"
RPM_ARCH="${RPM_ARCH:-}"

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temp_root=$(mktemp -d "${TMPDIR:-/tmp}/luminate-rpm-smoke.XXXXXX")
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

if [ "${BUILD_RPM}" = "1" ]; then
  "${repo_root}/packaging/build-rpm.sh"
fi

if { [ -n "${BUILD_TARGET}" ] && [ -z "${RPM_ARCH}" ]; } \
  || { [ -z "${BUILD_TARGET}" ] && [ -n "${RPM_ARCH}" ]; }; then
  printf '%s\n' "BUILD_TARGET and RPM_ARCH must be set together for a cross-built RPM" >&2
  exit 1
fi

if [ -n "${BUILD_TARGET}" ]; then
  rpm_dir="${repo_root}/target/${BUILD_TARGET}/generate-rpm"
else
  rpm_dir="${repo_root}/target/generate-rpm"
fi
set -- "${rpm_dir}"/*.rpm
if [ ! -f "$1" ]; then
  printf 'no .rpm found under %s; run packaging/build-rpm.sh first or set BUILD_RPM=1\n' "${rpm_dir}" >&2
  exit 1
fi
if [ "$#" -ne 1 ]; then
  printf '%s\n' "expected exactly one .rpm under target/generate-rpm; found $#" >&2
  exit 1
fi
rpm_path=$1

case "${RPM_ARCH}" in
  "")
    solibdir=$(rpm --eval '%{_libdir}')
    ;;
  i586|i686)
    solibdir=/usr/lib
    ;;
  *)
    printf 'unsupported cross-RPM architecture: %s\n' "${RPM_ARCH}" >&2
    exit 1
    ;;
esac
case "${solibdir}" in
  /usr/lib64)
    rpm_capability_suffix='()(64bit)'
    ;;
  /usr/lib)
    rpm_capability_suffix=
    ;;
  *)
    printf 'unsupported RPM %%_libdir: %s\n' "${solibdir}" >&2
    exit 1
    ;;
esac

abi_version=$(sed -n \
  's/^pub const LUMINATE_C_ABI_VERSION: u32 = \([0-9][0-9]*\);$/\1/p' \
  "${repo_root}/crates/libluminate/src/c_abi.rs")
if [ -z "${abi_version}" ]; then
  printf '%s\n' "could not read LUMINATE_C_ABI_VERSION from its canonical Rust definition" >&2
  exit 1
fi

rpm -qpl "${rpm_path}" \
  | grep -Fx "${solibdir}/libluminate.so.${abi_version}" >/dev/null
rpm -qpl "${rpm_path}" \
  | grep -Fx "${solibdir}/luminate/plugins/libluminate_plugin_govee.so" >/dev/null
rpm -qpl "${rpm_path}" \
  | grep -Fx "${solibdir}/luminate/plugins/libluminate_plugin_philips_hue.so" >/dev/null
rpm -qpl "${rpm_path}" \
  | grep -Fx "${solibdir}/luminate/plugins/libluminate_plugin_linux_leds.so" >/dev/null
rpm -qpl "${rpm_path}" \
  | grep -Fx "${solibdir}/luminate/plugins/libluminate_plugin_wled.so" >/dev/null
rpm -qpl "${rpm_path}" \
  | grep -Fx "${solibdir}/luminate/plugins/libluminate_plugin_razer.so" >/dev/null
rpm -qp --provides "${rpm_path}" \
  | grep -Fx "libluminate.so.${abi_version}${rpm_capability_suffix}" >/dev/null
rpm -qp --requires "${rpm_path}" \
  | grep -Fx "libluminate.so.${abi_version}${rpm_capability_suffix}" >/dev/null
rpm -qp --requires "${rpm_path}" | grep -Fx 'systemd' >/dev/null
rpm -qp --requires "${rpm_path}" | grep -Fx 'systemd-udev' >/dev/null
if [ "${INSTALL_DBUS:-0}" = "1" ]; then
  rpm -qp --requires "${rpm_path}" | grep -Fx 'dbus' >/dev/null
  rpm -qpl "${rpm_path}" | grep -Fx '/usr/bin/luminate-dbus' >/dev/null
  rpm -qpl "${rpm_path}" | grep -Fx '/usr/lib/systemd/system/luminate-dbus.service' >/dev/null
  rpm -qpl "${rpm_path}" | grep -Fx '/usr/lib/sysusers.d/luminate-dbus.conf' >/dev/null
  rpm -qpl "${rpm_path}" \
    | grep -Fx '/usr/share/dbus-1/system-services/org.luminate.Luminate1.service' >/dev/null
  rpm -qpl "${rpm_path}" \
    | grep -Fx '/usr/share/dbus-1/system.d/org.luminate.Luminate1.conf' >/dev/null
fi
if [ "${INSTALL_DBUS_POLKIT:-0}" = "1" ]; then
  rpm -qp --requires "${rpm_path}" | grep -Fx 'polkit' >/dev/null
fi
if [ "${INSTALL_HTTP:-0}" = "1" ]; then
  rpm -qpl "${rpm_path}" | grep -Fx '/usr/bin/luminate-http' >/dev/null
  rpm -qpl "${rpm_path}" | grep -Fx '/usr/lib/systemd/system/luminate-http.service' >/dev/null
  rpm -qpl "${rpm_path}" | grep -Fx '/usr/lib/sysusers.d/luminate-http.conf' >/dev/null
  rpm -qpl "${rpm_path}" | grep -Fx '/usr/lib/tmpfiles.d/luminate-http.conf' >/dev/null
fi
rpm -qp --scripts "${rpm_path}" \
  | grep -Fx "systemd-sysusers /usr/lib/sysusers.d/luminate.conf" >/dev/null
rpm -qp --scripts "${rpm_path}" \
  | grep -Fx "systemd-tmpfiles --create /usr/lib/tmpfiles.d/luminate.conf" >/dev/null

rpm_root="${temp_root}/rpm"
mkdir -p "${rpm_root}"
(
  cd "${rpm_root}"
  rpm2cpio "${rpm_path}" | cpio -idm --quiet
)
packaged_library="${rpm_root}${solibdir}/libluminate.so.${abi_version}"
readelf -d "${packaged_library}" \
  | grep -F "Library soname: [libluminate.so.${abi_version}]" >/dev/null
grep -Fx "#define LUMINATE_C_ABI_VERSION ${abi_version}" \
  "${rpm_root}/usr/include/luminate/luminate.h" >/dev/null

if [ "${VERIFY_RPM_ONLY}" = "1" ]; then
  printf '%s\n' "RPM C ABI consistency check passed"
  exit 0
fi

if [ "${BUILD_IMAGE}" = "1" ]; then
  "${CONTAINER_ENGINE}" build \
    -f "${repo_root}/packaging/rpm/Dockerfile" \
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
  while ! container_exec /usr/bin/luminatectl ping >/dev/null 2>&1 \
     || ! container_exec systemctl is-active --quiet luminated.service; do
    if [ "$(date +%s)" -ge "${deadline}" ]; then
      printf '%s\n' "timed out waiting for systemd-managed luminated" >&2
      dump_diagnostics
      exit 1
    fi
    sleep 0.1
  done
}

wait_for_cli
test "$(container_exec systemctl show -p User --value luminated.service)" = "luminated"
test "$(container_exec systemctl show -p Group --value luminated.service)" = "luminate"
container_exec rpm -q luminate >/dev/null

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

# Exercise the pre_uninstall scriptlet's `systemctl stop` path.
container_exec rpm -e luminate
if container_exec test -S /run/luminated/luminated.sock; then
  printf '%s\n' "rpm erase left the daemon socket behind" >&2
  exit 1
fi
if container_exec systemctl is-active --quiet luminated.service 2>/dev/null; then
  printf '%s\n' "luminated.service still active after rpm erase" >&2
  exit 1
fi

printf '%s\n' "rpm packaging smoke test passed"
