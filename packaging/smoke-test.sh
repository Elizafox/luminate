#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

# Smoke-test the packaged install layout without touching host system paths.
#
# The test builds and installs into SMOKE_ROOT, with all compiled defaults
# pointing at that runnable tree, then runs the installed daemon and client in
# two phases:
#
#   1. With the DEFAULT packaged config, asserting it exposes NO demo devices --
#      the demo plugin is deliberately kept off the default autoload path.
#   2. With the shipped example config (which opts into the demo plugin's
#      examples directory), asserting the demo devices are discovered and
#      mutable.

SMOKE_ROOT="${SMOKE_ROOT:-/tmp/luminate-smoke-root}"
PREFIX="${PREFIX:-${SMOKE_ROOT}/usr}"
BUILD_PROFILE="${BUILD_PROFILE:-debug}"
SMOKE_DBUS="${SMOKE_DBUS:-0}"
SMOKE_HTTP="${SMOKE_HTTP:-0}"

for smoke_toggle in "SMOKE_DBUS:${SMOKE_DBUS}" "SMOKE_HTTP:${SMOKE_HTTP}"; do
  case "${smoke_toggle#*:}" in
  0 | 1) ;;
  *)
    printf '%s must be 0 or 1\n' "${smoke_toggle%%:*}" >&2
    exit 1
    ;;
  esac
done
unset smoke_toggle

# Give the opt-in D-Bus case a private bus. The bridge deliberately connects
# to the system-bus address, which the child maps to this private session.
if [ "${SMOKE_DBUS}" = "1" ] && [ "${DBUS_SMOKE_CHILD:-0}" != "1" ]; then
  command -v dbus-run-session >/dev/null 2>&1 || {
    printf '%s\n' "SMOKE_DBUS=1 requires dbus-run-session" >&2
    exit 1
  }
  DBUS_SMOKE_CHILD=1 exec dbus-run-session -- "$0"
fi

CONFIG_PATH="${CONFIG_PATH:-${SMOKE_ROOT}/etc/luminate/luminated.toml}"
RUNTIME_DIR="${RUNTIME_DIR:-${SMOKE_ROOT}/run/luminated}"
SOCKET_PATH="${SOCKET_PATH:-${RUNTIME_DIR}/luminated.sock}"
STATE_DIR="${STATE_DIR:-${SMOKE_ROOT}/var/lib/luminated}"
STATE_PATH="${STATE_PATH:-${STATE_DIR}/state.json}"
HTTP_STATE_DIR="${HTTP_STATE_DIR:-${SMOKE_ROOT}/var/lib/luminate-http}"
HTTP_LISTEN="${HTTP_LISTEN:-127.0.0.1:18080}"
SMOKE_FRONTEND_USER="${SMOKE_FRONTEND_USER:-$(id -un)}"
PLUGIN_DIR_LOCAL="${PLUGIN_DIR_LOCAL:-${SMOKE_ROOT}/usr/local/lib/luminate/plugins}"
PLUGIN_DIR_SYSTEM="${PLUGIN_DIR_SYSTEM:-${PREFIX}/lib/luminate/plugins}"
EXAMPLE_CONFIG_PATH="${EXAMPLE_CONFIG_PATH:-${PREFIX}/share/luminate/examples/luminated-demo.toml}"

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

# Invalid overrides must be rejected before a build or any privileged template
# can consume them. These cover each grammar hazard shared by TOML, systemd,
# sysusers, and tmpfiles.
for unsafe_path in \
  '/tmp/luminate path' \
  '/tmp/luminate"path' \
  '/tmp/luminate\path' \
  '/tmp/luminate%h' \
  '/tmp/luminate;path'; do
  if RUNTIME_DIR="${unsafe_path}" BUILD_PROFILE=debug \
    "${repo_root}/packaging/install-linux.sh" >/dev/null 2>&1; then
    printf 'unsafe packaging path was accepted: %s\n' "${unsafe_path}" >&2
    exit 1
  fi
done

rm -rf "${SMOKE_ROOT}"

DESTDIR="" \
PREFIX="${PREFIX}" \
SYSCONFDIR="${SMOKE_ROOT}/etc" \
LOCALSTATEDIR="${SMOKE_ROOT}/var" \
RUNDIR="${SMOKE_ROOT}/run" \
CONFIG_PATH="${CONFIG_PATH}" \
RUNTIME_DIR="${RUNTIME_DIR}" \
SOCKET_PATH="${SOCKET_PATH}" \
STATE_DIR="${STATE_DIR}" \
STATE_PATH="${STATE_PATH}" \
PLUGIN_DIR_LOCAL="${PLUGIN_DIR_LOCAL}" \
PLUGIN_DIR_SYSTEM="${PLUGIN_DIR_SYSTEM}" \
BUILD_PROFILE="${BUILD_PROFILE}" \
INSTALL_DBUS="${SMOKE_DBUS}" \
INSTALL_HTTP="${SMOKE_HTTP}" \
LUMINATE_DBUS_USER="${SMOKE_FRONTEND_USER}" \
LUMINATE_HTTP_USER="${SMOKE_FRONTEND_USER}" \
HTTP_STATE_DIR="${HTTP_STATE_DIR}" \
HTTP_LISTEN="${HTTP_LISTEN}" \
"${repo_root}/packaging/install-linux.sh"

if [ ! -f "${PLUGIN_DIR_SYSTEM}/libluminate_plugin_lifx.so" ]; then
  printf '%s\n' "packaged LIFX plugin is missing" >&2
  exit 1
fi

if [ ! -f "${PLUGIN_DIR_SYSTEM}/libluminate_plugin_philips_hue.so" ]; then
  printf '%s\n' "packaged Philips Hue plugin is missing" >&2
  exit 1
fi

if [ ! -f "${PLUGIN_DIR_SYSTEM}/libluminate_plugin_linux_leds.so" ]; then
  printf '%s\n' "packaged Linux LED-class plugin is missing" >&2
  exit 1
fi

if [ ! -f "${PLUGIN_DIR_SYSTEM}/libluminate_plugin_wled.so" ]; then
  printf '%s\n' "packaged WLED plugin is missing" >&2
  exit 1
fi

if [ ! -f "${PLUGIN_DIR_SYSTEM}/libluminate_plugin_razer.so" ]; then
  printf '%s\n' "packaged Razer plugin is missing" >&2
  exit 1
fi

if [ ! -f "${SMOKE_ROOT}/usr/lib/udev/rules.d/60-luminate-alienware.rules" ]; then
  printf '%s\n' "packaged Alienware udev rule is missing" >&2
  exit 1
fi

for alienware_id in '0d62 d2b1' '187c 0550' '187c 0551'; do
  alienware_vendor=${alienware_id% *}
  alienware_product=${alienware_id#* }
  if ! grep -F "ATTRS{idVendor}==\"${alienware_vendor}\", ATTRS{idProduct}==\"${alienware_product}\"" \
    "${SMOKE_ROOT}/usr/lib/udev/rules.d/60-luminate-alienware.rules" >/dev/null; then
    printf 'packaged Alienware udev rule is missing %s:%s\n' \
      "${alienware_vendor}" "${alienware_product}" >&2
    exit 1
  fi
done

if [ ! -f "${SMOKE_ROOT}/usr/lib/udev/rules.d/60-luminate-razer.rules" ]; then
  printf '%s\n' "packaged Razer udev rule is missing" >&2
  exit 1
fi

if [ ! -f "${SMOKE_ROOT}/usr/lib/udev/rules.d/60-luminate-linux-leds.rules" ]; then
  printf '%s\n' "packaged Linux LED-class udev rule is missing" >&2
  exit 1
fi

if ! grep -Fx 'activation = "host-attached"' "${CONFIG_PATH}" >/dev/null; then
  printf '%s\n' "packaged config does not select host-attached plugin activation" >&2
  exit 1
fi

mkdir -p "${RUNTIME_DIR}" "${STATE_DIR}"

# The D-Bus bridge delegates each bus caller into an authenticated daemon
# session. Seed the isolated smoke state with an explicit policy for this
# process's UID; the daemon's authenticated-subject policy is default-deny and
# must not inherit the bridge's peer-authenticated socket trust.
if [ "${SMOKE_DBUS}" = "1" ]; then
  smoke_uid=$(id -u)
  (
    umask 077
    printf '%s\n' \
      '{' \
      '  "revision": 1,' \
      '  "roles": {' \
      '    "dbus-smoke": {' \
      '      "rules": [{' \
      '        "id": "dbus-smoke-control",' \
      '        "effect": "allow",' \
      '        "operations": ["observe", "control"],' \
      '        "reason": null,' \
      '        "cache_hint": null' \
      '      }]' \
      '    }' \
      '  },' \
      '  "bindings": [{' \
      '    "authority": "unix",' \
      "    \"subjects\": [\"${smoke_uid}\"]," \
      '    "groups": [],' \
      '    "roles": ["dbus-smoke"]' \
      '  }]' \
      '}' >"${STATE_DIR}/policy.json"
  )
fi

daemon="${PREFIX}/bin/luminated"
cli="${PREFIX}/bin/luminatectl"
log_path="${SMOKE_ROOT}/luminated-smoke.log"

daemon_pid=""
dbus_pid=""
http_pid=""
cleanup() {
  if [ -n "${http_pid}" ]; then
    kill "${http_pid}" 2>/dev/null || true
    wait "${http_pid}" 2>/dev/null || true
  fi
  if [ -n "${dbus_pid}" ]; then
    kill "${dbus_pid}" 2>/dev/null || true
    wait "${dbus_pid}" 2>/dev/null || true
  fi
  if [ -n "${daemon_pid}" ]; then
    kill "${daemon_pid}" 2>/dev/null || true
    wait "${daemon_pid}" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

# Block until the daemon's socket appears, dumping the log and failing on
# timeout.
wait_for_socket() {
  deadline=$(($(date +%s) + 10))
  while [ ! -S "${SOCKET_PATH}" ]; do
    if [ "$(date +%s)" -ge "${deadline}" ]; then
      printf '%s\n' "timed out waiting for ${SOCKET_PATH}" >&2
      printf '%s\n' "--- daemon log ---" >&2
      cat "${log_path}" >&2 || true
      exit 1
    fi
    sleep 0.05
  done
}

# Stop the running daemon and clear its socket so the next phase starts clean.
stop_daemon() {
  if [ -n "${daemon_pid}" ]; then
    kill "${daemon_pid}" 2>/dev/null || true
    wait "${daemon_pid}" 2>/dev/null || true
    daemon_pid=""
  fi
  rm -f "${SOCKET_PATH}"
}

# Phase 1: the DEFAULT packaged config must not expose the demo plugin's fake
# devices, since the demo plugin lives outside the default autoload path.
"${daemon}" >"${log_path}" 2>&1 &
daemon_pid=$!
wait_for_socket
"${cli}" version >/dev/null

default_list_json=$("${cli}" list --json)
if printf '%s\n' "${default_list_json}" | grep -Fq '"id": "demo-'; then
  printf '%s\n' "default config unexpectedly exposed a demo device:" >&2
  printf '%s\n' "${default_list_json}" >&2
  exit 1
fi

stop_daemon

# Phase 2: with the shipped example config (which opts into the examples
# directory), the demo devices must be discovered and mutable.
LUMINATED_CONFIG="${EXAMPLE_CONFIG_PATH}" "${daemon}" >"${log_path}" 2>&1 &
daemon_pid=$!
wait_for_socket
"${cli}" version >/dev/null

list_json=$("${cli}" list --json)
printf '%s\n' "${list_json}" | grep -F '"id": "demo-keyboard"' >/dev/null
printf '%s\n' "${list_json}" | grep -F '"id": "demo-case-lights"' >/dev/null

mutation_output=$("${cli}" set-effect --effect static --device demo-keyboard --surface zones --element g1 --rgb 'rgb(12, 34, 56)')
if [ "${mutation_output}" != "ok" ]; then
  printf 'unexpected mutation output: %s\n' "${mutation_output}" >&2
  exit 1
fi

# Optional phase 3: exercise the packaged D-Bus bridge on the private bus.
if [ "${SMOKE_DBUS}" = "1" ]; then
  command -v gdbus >/dev/null 2>&1 || {
    printf '%s\n' "SMOKE_DBUS=1 requires gdbus" >&2
    exit 1
  }
  dbus_bridge="${PREFIX}/bin/luminate-dbus"
  if [ ! -x "${dbus_bridge}" ]; then
    printf '%s\n' "packaged D-Bus bridge is missing" >&2
    exit 1
  fi
  if [ ! -f "${PREFIX}/share/dbus-1/system-services/org.luminate.Luminate1.service" ] ||
     [ ! -f "${PREFIX}/share/dbus-1/system.d/org.luminate.Luminate1.conf" ]; then
    printf '%s\n' "packaged D-Bus activation or policy file is missing" >&2
    exit 1
  fi

  group_file="${SMOKE_ROOT}/group"
  printf 'luminate:x:%s:\n' "$(id -g)" >"${group_file}"
  DBUS_SYSTEM_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS}" \
    "${dbus_bridge}" --socket-path "${SOCKET_PATH}" --group-file "${group_file}" \
    >"${SMOKE_ROOT}/luminate-dbus-smoke.log" 2>&1 &
  dbus_pid=$!

  deadline=$(($(date +%s) + 10))
  while ! managed_objects=$(gdbus call \
      --address "${DBUS_SESSION_BUS_ADDRESS}" \
      --dest org.luminate.Luminate1 \
      --object-path /org/luminate/Luminate1 \
      --method org.freedesktop.DBus.ObjectManager.GetManagedObjects 2>/dev/null); do
    if [ "$(date +%s)" -ge "${deadline}" ]; then
      printf '%s\n' "timed out waiting for packaged D-Bus bridge" >&2
      cat "${SMOKE_ROOT}/luminate-dbus-smoke.log" >&2 || true
      exit 1
    fi
    sleep 0.05
  done
  printf '%s\n' "${managed_objects}" | grep -F 'org.luminate.Luminate1.Device1' >/dev/null
  printf '%s\n' "${managed_objects}" | grep -F 'device:demo-keyboard' >/dev/null

  dbus_mutation=$(gdbus call \
    --address "${DBUS_SESSION_BUS_ADDRESS}" \
    --dest org.luminate.Luminate1 \
    --object-path /org/luminate/Luminate1/devices/x64656d6f2d6b6579626f617264/surfaces/x7a6f6e6573/elements/x6731 \
    --method org.luminate.Target2.SetEffect \
    "{'Kind': <'static'>, 'StaticColour': <{'Model': <'additive'>, 'Channels': <{'red': uint32 12, 'green': uint32 34, 'blue': uint32 56}>}>}")
  if [ "${dbus_mutation}" != "()" ]; then
    printf 'unexpected D-Bus mutation output: %s\n' "${dbus_mutation}" >&2
    exit 1
  fi
fi

# Optional phase 4: start the packaged HTTP front end, provision its first
# bearer token through the offline management CLI, and exercise authentication.
if [ "${SMOKE_HTTP}" = "1" ]; then
  command -v curl >/dev/null 2>&1 || {
    printf '%s\n' "SMOKE_HTTP=1 requires curl" >&2
    exit 1
  }
  http="${PREFIX}/bin/luminate-http"
  if [ ! -x "${http}" ]; then
    printf '%s\n' "packaged HTTP front end is missing" >&2
    exit 1
  fi
  if [ ! -f "${SMOKE_ROOT}/usr/lib/systemd/system/luminate-http.service" ] ||
     [ ! -f "${SMOKE_ROOT}/usr/lib/sysusers.d/luminate-http.conf" ] ||
     [ ! -f "${SMOKE_ROOT}/usr/lib/tmpfiles.d/luminate-http.conf" ]; then
    printf '%s\n' "packaged HTTP service integration is incomplete" >&2
    exit 1
  fi

  http_certificate="${repo_root}/crates/luminate-http/testdata/tls/dns-cert.pem"
  http_private_key="${SMOKE_ROOT}/luminate-http-smoke.key"
  cp "${repo_root}/crates/luminate-http/testdata/tls/dns-key.pem" "${http_private_key}"
  chmod 600 "${http_private_key}"
  http_secret='c2VjcmV0LXJlZGFjdGlvbi1zbW9rZQ'

  "${http}" --listen "${HTTP_LISTEN}" --socket-path "${SOCKET_PATH}" \
    --tls-certificate "${http_certificate}" --tls-private-key "${http_private_key}" \
    >"${SMOKE_ROOT}/luminate-http-smoke.log" 2>&1 &
  http_pid=$!

  deadline=$(($(date +%s) + 10))
  while ! ready_json=$(curl --insecure --fail --silent "https://${HTTP_LISTEN}/health/ready"); do
    if [ "$(date +%s)" -ge "${deadline}" ]; then
      printf '%s\n' "timed out waiting for packaged HTTP front end" >&2
      cat "${SMOKE_ROOT}/luminate-http-smoke.log" >&2 || true
      exit 1
    fi
    sleep 0.05
  done
  printf '%s\n' "${ready_json}" | grep -F '"status":"ready"' >/dev/null
  curl --insecure --fail --silent --tlsv1.2 --tls-max 1.2 \
    "https://${HTTP_LISTEN}/health/ready" >/dev/null
  curl --insecure --fail --silent --tlsv1.3 --tls-max 1.3 \
    "https://${HTTP_LISTEN}/health/ready" >/dev/null
  if curl --silent --tls-max 1.1 "https://${HTTP_LISTEN}/health/live" >/dev/null 2>&1; then
    printf '%s\n' "obsolete TLS unexpectedly reached the HTTP listener" >&2
    exit 1
  fi
  if curl --silent "http://${HTTP_LISTEN}/health/live" >/dev/null 2>&1; then
    printf '%s\n' "plaintext unexpectedly reached the TLS HTTP listener" >&2
    exit 1
  fi
  curl --insecure --silent -H "Authorization: Bearer ${http_secret}" \
    "https://${HTTP_LISTEN}/api/v0/me" >/dev/null
  if grep -F "${http_secret}" "${SMOKE_ROOT}/luminate-http-smoke.log" >/dev/null; then
    printf '%s\n' "HTTP log exposed a bearer secret" >&2
    exit 1
  fi
fi

printf '%s\n' "packaging smoke test passed"
