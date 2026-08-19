#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

# Build and install the macOS daemon, CLI, and portable plugins from source.
# This is intentionally separate from install-linux.sh: launchd integration,
# Darwin dynamic libraries, and macOS's machine-wide paths share no useful
# packaging implementation with the Linux distro installer.

if [ "$(uname -s)" != "Darwin" ]; then
  printf '%s\n' "error: packaging/install-macos.sh must run on macOS" >&2
  exit 1
fi

DESTDIR="${DESTDIR:-}"
BUILD_PROFILE="${BUILD_PROFILE:-release}"
BUILD_TARGET="${BUILD_TARGET:-}"
START_SERVICE="${START_SERVICE:-0}"

INSTALL_DEMO_PLUGIN="${INSTALL_DEMO_PLUGIN:-1}"
INSTALL_LIFX_PLUGIN="${INSTALL_LIFX_PLUGIN:-1}"
INSTALL_PHILIPS_HUE_PLUGIN="${INSTALL_PHILIPS_HUE_PLUGIN:-1}"
INSTALL_WLED_PLUGIN="${INSTALL_WLED_PLUGIN:-1}"
INSTALL_GOVEE_PLUGIN="${INSTALL_GOVEE_PLUGIN:-1}"

BINDIR="/usr/local/bin"
PLUGIN_DIR_LOCAL="/usr/local/lib/luminate/plugins"
PLUGIN_DIR_EXAMPLES="/usr/local/lib/luminate/plugins-examples"

CONFIG_PATH="/Library/Application Support/Luminate/luminated.toml"
SOCKET_PATH="/var/run/luminated/luminated.sock"
STATE_PATH="/Library/Application Support/Luminate/state/state.json"
LOG_PATH="/Library/Logs/luminated/luminated.log"
PLUGIN_DIR_SYSTEM="/Library/Application Support/Luminate/plugins"

validate_toggle() {
  name=$1
  value=$2
  case "${value}" in
  0 | 1) ;;
  *)
    printf 'error: %s must be 0 or 1: %s\n' "${name}" "${value}" >&2
    exit 1
    ;;
  esac
}

validate_toggle START_SERVICE "${START_SERVICE}"
validate_toggle INSTALL_DEMO_PLUGIN "${INSTALL_DEMO_PLUGIN}"
validate_toggle INSTALL_LIFX_PLUGIN "${INSTALL_LIFX_PLUGIN}"
validate_toggle INSTALL_PHILIPS_HUE_PLUGIN "${INSTALL_PHILIPS_HUE_PLUGIN}"
validate_toggle INSTALL_WLED_PLUGIN "${INSTALL_WLED_PLUGIN}"
validate_toggle INSTALL_GOVEE_PLUGIN "${INSTALL_GOVEE_PLUGIN}"

case "${DESTDIR}" in
"") ;;
/*) ;;
*)
  printf 'error: DESTDIR must be empty or an absolute path: %s\n' "${DESTDIR}" >&2
  exit 1
  ;;
esac

if [ -z "${BUILD_PROFILE}" ]; then
  printf '%s\n' "error: BUILD_PROFILE must not be empty" >&2
  exit 1
fi

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "${repo_root}"

export LUMINATE_DEFAULT_CONFIG_PATH="${CONFIG_PATH}"
export LUMINATE_DEFAULT_SOCKET_PATH="${SOCKET_PATH}"
export LUMINATE_DEFAULT_STATE_PATH="${STATE_PATH}"
export LUMINATE_DEFAULT_PLUGIN_DIR_LOCAL="${PLUGIN_DIR_LOCAL}"
export LUMINATE_DEFAULT_PLUGIN_DIR_SYSTEM="${PLUGIN_DIR_SYSTEM}"

set -- cargo build
if [ -n "${BUILD_TARGET}" ]; then
  set -- "$@" --target "${BUILD_TARGET}"
fi
if [ "${BUILD_PROFILE}" = "release" ]; then
  set -- "$@" --release
elif [ "${BUILD_PROFILE}" != "debug" ] && [ "${BUILD_PROFILE}" != "dev" ]; then
  set -- "$@" --profile "${BUILD_PROFILE}"
fi
set -- "$@" -p luminated -p luminate-cli
if [ "${INSTALL_DEMO_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-demo-system
fi
if [ "${INSTALL_LIFX_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-lifx
fi
if [ "${INSTALL_PHILIPS_HUE_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-philips-hue
fi
if [ "${INSTALL_WLED_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-wled
fi
if [ "${INSTALL_GOVEE_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-govee
fi
"$@"

if [ "${BUILD_PROFILE}" = "release" ]; then
  target_profile_dir="release"
elif [ "${BUILD_PROFILE}" = "debug" ] || [ "${BUILD_PROFILE}" = "dev" ]; then
  target_profile_dir="debug"
else
  target_profile_dir="${BUILD_PROFILE}"
fi

if [ -n "${BUILD_TARGET}" ]; then
  target_dir="${repo_root}/target/${BUILD_TARGET}/${target_profile_dir}"
else
  target_dir="${repo_root}/target/${target_profile_dir}"
fi

if [ -z "${DESTDIR}" ] && [ "$(id -u)" -ne 0 ] && ! command -v sudo >/dev/null 2>&1; then
  printf '%s\n' "error: sudo is required for a live install when not running as root" >&2
  exit 1
fi

run_privileged() {
  if [ -n "${DESTDIR}" ] || [ "$(id -u)" -eq 0 ]; then
    "$@"
  else
    sudo "$@"
  fi
}

wait_for_socket() {
  attempts=0
  while [ "${attempts}" -lt 100 ]; do
    if run_privileged test -S "${SOCKET_PATH}"; then
      return 0
    fi
    attempts=$((attempts + 1))
    sleep 0.1
  done

  printf 'error: launchd started luminated but its socket did not appear: %s\n' \
    "${SOCKET_PATH}" >&2
  return 1
}

install_directory() {
  run_privileged install -d -m 0755 "${DESTDIR}$1"
}

install_file() {
  source_path=$1
  target_path=$2
  run_privileged install -m 0755 "${source_path}" "${DESTDIR}${target_path}"
}

install_directory "${BINDIR}"
install_directory "${PLUGIN_DIR_LOCAL}"

install_file "${target_dir}/luminated" "${BINDIR}/luminated"
install_file "${target_dir}/luminatectl" "${BINDIR}/luminatectl"

if [ "${INSTALL_DEMO_PLUGIN}" = "1" ]; then
  install_directory "${PLUGIN_DIR_EXAMPLES}"
  install_file \
    "${target_dir}/libluminate_plugin_demo_system.dylib" \
    "${PLUGIN_DIR_EXAMPLES}/libluminate_plugin_demo_system.dylib"
fi
if [ "${INSTALL_LIFX_PLUGIN}" = "1" ]; then
  install_file \
    "${target_dir}/libluminate_plugin_lifx.dylib" \
    "${PLUGIN_DIR_LOCAL}/libluminate_plugin_lifx.dylib"
fi
if [ "${INSTALL_PHILIPS_HUE_PLUGIN}" = "1" ]; then
  install_file \
    "${target_dir}/libluminate_plugin_philips_hue.dylib" \
    "${PLUGIN_DIR_LOCAL}/libluminate_plugin_philips_hue.dylib"
fi
if [ "${INSTALL_WLED_PLUGIN}" = "1" ]; then
  install_file \
    "${target_dir}/libluminate_plugin_wled.dylib" \
    "${PLUGIN_DIR_LOCAL}/libluminate_plugin_wled.dylib"
fi
if [ "${INSTALL_GOVEE_PLUGIN}" = "1" ]; then
  install_file \
    "${target_dir}/libluminate_plugin_govee.dylib" \
    "${PLUGIN_DIR_LOCAL}/libluminate_plugin_govee.dylib"
fi

if [ -n "${DESTDIR}" ]; then
  printf 'Staged macOS Luminate files under %s\n' "${DESTDIR}"
  printf '%s\n' "Skipped service registration because DESTDIR is set."
  exit 0
fi

service_state=$(run_privileged "${BINDIR}/luminated" service status)
if [ "${START_SERVICE}" = "1" ] && [ "${service_state}" = "not-loaded" ]; then
  run_privileged "${BINDIR}/luminated" service install --start
elif [ "${START_SERVICE}" = "1" ]; then
  run_privileged "${BINDIR}/luminated" service install
  run_privileged "${BINDIR}/luminated" service start
else
  run_privileged "${BINDIR}/luminated" service install
fi

if [ "${START_SERVICE}" = "1" ]; then
  wait_for_socket
fi

printf '%s\n' "Installed Luminate for macOS:"
printf '  daemon: %s\n' "${BINDIR}/luminated"
printf '  client: %s\n' "${BINDIR}/luminatectl"
printf '  autoloaded plugins: %s\n' "${PLUGIN_DIR_LOCAL}"
printf '  example plugins: %s\n' "${PLUGIN_DIR_EXAMPLES}"
printf '  config: %s\n' "${CONFIG_PATH}"
printf '  socket: %s\n' "${SOCKET_PATH}"
printf '  state: %s\n' "${STATE_PATH}"
printf '  log: %s\n' "${LOG_PATH}"
printf '  system plugins: %s\n' "${PLUGIN_DIR_SYSTEM}"
if [ "${START_SERVICE}" = "1" ]; then
  printf '%s\n' "  launchd service: started"
elif [ "${service_state}" = "running" ]; then
  printf '%s\n' "  launchd service: still running; restart it to use updated files"
else
  printf '%s\n' "  launchd service: registered for the next boot"
fi
printf '%s\n' \
  "Client users must belong to _luminate. Add one with:" \
  "  sudo dseditgroup -o edit -a <user> -t user _luminate" \
  "Then log out and back in before running luminatectl."
