#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

# Build and install Luminate into a Linux packaging root.
#
# This script treats install paths as build-time configuration: it passes the
# selected paths through LUMINATE_DEFAULT_* while compiling, then renders the
# packaged config/service files with the same values. Runtime path overrides are
# still useful for development, but packaged installs should not need them.

DESTDIR="${DESTDIR:-}"
PREFIX="${PREFIX:-/usr}"
SYSCONFDIR="${SYSCONFDIR:-/etc}"
LOCALSTATEDIR="${LOCALSTATEDIR:-/var}"
RUNDIR="${RUNDIR:-/run}"
LIBDIR="${LIBDIR:-${PREFIX}/lib}"
BINDIR="${BINDIR:-${PREFIX}/bin}"
INCLUDEDIR="${INCLUDEDIR:-${PREFIX}/include}"
# Where ELF shared objects (libluminate.so.* and the plugin .so's) actually
# land, as opposed to LIBDIR, which also holds arch-independent data
# (systemd units, sysusers/tmpfiles/udev config) that stays under /usr/lib
# even on 64-bit multilib distros. Defaults to LIBDIR so non-multilib
# targets (deb, Alpine, OpenRC) are unaffected; RPM builds on 64-bit Fedora
# override this to /usr/lib64.
SOLIBDIR="${SOLIBDIR:-${LIBDIR}}"

LUMINATE_USER="${LUMINATE_USER:-luminated}"
LUMINATE_GROUP="${LUMINATE_GROUP:-luminate}"
# The optional D-Bus bridge gets its own service account rather than running
# as root or as the daemon account, so daemon-side policy can grant it an
# explicit, scoped ceiling (see "D-Bus and Polkit" in
# docs/development/architecture/client-authorization.md). It joins
# LUMINATE_GROUP so it can reach the control socket like any other named
# client.
LUMINATE_DBUS_USER="${LUMINATE_DBUS_USER:-luminate-dbus}"
# The optional HTTP front end is isolated from both the daemon and other
# front ends. Its membership in the client group grants socket traversal, not
# daemon authorization; the daemon's policy remains the hard ceiling.
LUMINATE_HTTP_USER="${LUMINATE_HTTP_USER:-luminate-http}"

CONFIG_DIR="${CONFIG_DIR:-${SYSCONFDIR}/luminate}"
CONFIG_PATH="${CONFIG_PATH:-${CONFIG_DIR}/luminated.toml}"
RUNTIME_DIR="${RUNTIME_DIR:-${RUNDIR}/luminated}"
SOCKET_PATH="${SOCKET_PATH:-${RUNTIME_DIR}/luminated.sock}"
STATE_DIR="${STATE_DIR:-${LOCALSTATEDIR}/lib/luminated}"
STATE_PATH="${STATE_PATH:-${STATE_DIR}/state.json}"
HTTP_STATE_DIR="${HTTP_STATE_DIR:-${LOCALSTATEDIR}/lib/luminate-http}"
HTTP_LISTEN="${HTTP_LISTEN:-127.0.0.1:8080}"
PLUGIN_DIR_LOCAL="${PLUGIN_DIR_LOCAL:-/usr/local/lib/luminate/plugins}"
PLUGIN_DIR_SYSTEM="${PLUGIN_DIR_SYSTEM:-${SOLIBDIR}/luminate/plugins}"
# The bundled demo plugin exposes fake devices; it is deliberately installed
# outside the default autoload path (PLUGIN_DIR_LOCAL/PLUGIN_DIR_SYSTEM) so a
# production install never surfaces it. Operators opt in via EXAMPLE_CONFIG_PATH.
PLUGIN_DIR_EXAMPLES="${PLUGIN_DIR_EXAMPLES:-${SOLIBDIR}/luminate/plugins-examples}"
# Arch-independent app data, not a library: lives under PREFIX/share rather
# than LIBDIR/SOLIBDIR so it survives regardless of lib/lib64 layout, and
# isn't silently dropped by doc-stripped installs the way
# PREFIX/share/doc/... contents can be (Debian's doc-path excludes, RPM's
# %_excludedocs) -- this file is live opt-in config, not just documentation.
EXAMPLE_CONFIG_PATH="${EXAMPLE_CONFIG_PATH:-${PREFIX}/share/luminate/examples/luminated-demo.toml}"

SYSTEMD_UNIT_DIR="${SYSTEMD_UNIT_DIR:-${LIBDIR}/systemd/system}"
SYSUSERS_DIR="${SYSUSERS_DIR:-${LIBDIR}/sysusers.d}"
TMPFILES_DIR="${TMPFILES_DIR:-${LIBDIR}/tmpfiles.d}"
UDEV_RULES_DIR="${UDEV_RULES_DIR:-${LIBDIR}/udev/rules.d}"
OPENRC_INIT_DIR="${OPENRC_INIT_DIR:-${SYSCONFDIR}/init.d}"
OPENRC_CONF_DIR="${OPENRC_CONF_DIR:-${SYSCONFDIR}/conf.d}"
DBUS_SYSTEM_SERVICES_DIR="${DBUS_SYSTEM_SERVICES_DIR:-${PREFIX}/share/dbus-1/system-services}"
DBUS_SYSTEM_POLICY_DIR="${DBUS_SYSTEM_POLICY_DIR:-${PREFIX}/share/dbus-1/system.d}"
POLKIT_ACTIONS_DIR="${POLKIT_ACTIONS_DIR:-${PREFIX}/share/polkit-1/actions}"

# Keep the core install independent from any one service manager. Distro
# packages select the integration they ship; "none" is useful for containers
# and custom supervisors.
INIT_SYSTEM="${INIT_SYSTEM:-systemd}"

# Hardware backends remain independently packageable. Defaults preserve the
# existing all-in-one install, while Alpine/LIFX-only packages can avoid the
# Alienware plugin's eudev dependency without losing native plugin support.
INSTALL_DEMO_PLUGIN="${INSTALL_DEMO_PLUGIN:-1}"
INSTALL_ALIENWARE_PLUGIN="${INSTALL_ALIENWARE_PLUGIN:-1}"
INSTALL_RAZER_PLUGIN="${INSTALL_RAZER_PLUGIN:-1}"
INSTALL_LIFX_PLUGIN="${INSTALL_LIFX_PLUGIN:-1}"
INSTALL_PHILIPS_HUE_PLUGIN="${INSTALL_PHILIPS_HUE_PLUGIN:-1}"
INSTALL_GOVEE_PLUGIN="${INSTALL_GOVEE_PLUGIN:-1}"
INSTALL_LINUX_LEDS_PLUGIN="${INSTALL_LINUX_LEDS_PLUGIN:-1}"
INSTALL_WLED_PLUGIN="${INSTALL_WLED_PLUGIN:-1}"
# The desktop bridge is a separate component and is deliberately off by
# default, so base daemon/library and Alpine/OpenRC builds acquire no D-Bus
# dependency. Distro split packages opt in explicitly.
INSTALL_DBUS="${INSTALL_DBUS:-0}"
INSTALL_DBUS_POLKIT="${INSTALL_DBUS_POLKIT:-0}"
# The HTTP API is a separate, pre-release component. Keep it opt-in so an
# ordinary install does not unexpectedly open a TCP listener.
INSTALL_HTTP="${INSTALL_HTTP:-0}"

# These are all substituted into rendered template files and, for the
# user/group, into generated sysusers/systemd unit content. None of them may
# be trusted to be shell- or sed-safe: reject anything that isn't a plain
# absolute path or a plain POSIX account name up front, rather than relying on
# quoting alone downstream.
validate_abs_path() {
  name=$1
  value=$2
  # One value is rendered into TOML strings, systemd command/directive words,
  # and sysusers/tmpfiles fields. Restrict it to the portable path characters
  # that are literal in every one of those grammars; being absolute alone is
  # not sufficient (quotes, backslashes, whitespace, and systemd '%' all have
  # grammar-level meaning).
  if ! printf '%s' "${value}" | LC_ALL=C grep -Eq '^/[A-Za-z0-9._/+~-]*$'; then
    printf 'error: %s must be an absolute path using only portable safe characters: %s\n' \
      "${name}" "${value}" >&2
    exit 1
  fi
}

validate_account_name() {
  name=$1
  value=$2
  # Shell `case` glob patterns don't support "repeated character class" the
  # way a regex `[...]*` does -- a trailing `*` matches any characters at
  # all, not just more of the preceding bracket class -- so this has to be a
  # real anchored regex via grep, not a case/glob match.
  if ! printf '%s' "${value}" | LC_ALL=C grep -Eq '^[a-z_][a-z0-9_-]*$'; then
    printf 'error: %s is not a valid POSIX account/group name: %s\n' "${name}" "${value}" >&2
    exit 1
  fi
}

for _pair in \
  "BINDIR:${BINDIR}" "LIBDIR:${LIBDIR}" "SOLIBDIR:${SOLIBDIR}" "INCLUDEDIR:${INCLUDEDIR}" \
  "CONFIG_DIR:${CONFIG_DIR}" "CONFIG_PATH:${CONFIG_PATH}" \
  "RUNTIME_DIR:${RUNTIME_DIR}" "SOCKET_PATH:${SOCKET_PATH}" \
  "STATE_DIR:${STATE_DIR}" "STATE_PATH:${STATE_PATH}" \
  "HTTP_STATE_DIR:${HTTP_STATE_DIR}" \
  "PLUGIN_DIR_LOCAL:${PLUGIN_DIR_LOCAL}" "PLUGIN_DIR_SYSTEM:${PLUGIN_DIR_SYSTEM}" \
  "PLUGIN_DIR_EXAMPLES:${PLUGIN_DIR_EXAMPLES}" "EXAMPLE_CONFIG_PATH:${EXAMPLE_CONFIG_PATH}" \
  "UDEV_RULES_DIR:${UDEV_RULES_DIR}"; do
  validate_abs_path "${_pair%%:*}" "${_pair#*:}"
done
unset _pair
validate_account_name LUMINATE_USER "${LUMINATE_USER}"
validate_account_name LUMINATE_GROUP "${LUMINATE_GROUP}"
validate_account_name LUMINATE_DBUS_USER "${LUMINATE_DBUS_USER}"
validate_account_name LUMINATE_HTTP_USER "${LUMINATE_HTTP_USER}"

case "${INIT_SYSTEM}" in
systemd)
  for _pair in \
    "SYSTEMD_UNIT_DIR:${SYSTEMD_UNIT_DIR}" "SYSUSERS_DIR:${SYSUSERS_DIR}" \
    "TMPFILES_DIR:${TMPFILES_DIR}"; do
    validate_abs_path "${_pair%%:*}" "${_pair#*:}"
  done
  unset _pair
  ;;
openrc)
  validate_abs_path OPENRC_INIT_DIR "${OPENRC_INIT_DIR}"
  validate_abs_path OPENRC_CONF_DIR "${OPENRC_CONF_DIR}"
  ;;
none) ;;
*)
  printf 'error: INIT_SYSTEM must be systemd, openrc, or none: %s\n' "${INIT_SYSTEM}" >&2
  exit 1
  ;;
esac

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

validate_toggle INSTALL_DEMO_PLUGIN "${INSTALL_DEMO_PLUGIN}"
validate_toggle INSTALL_ALIENWARE_PLUGIN "${INSTALL_ALIENWARE_PLUGIN}"
validate_toggle INSTALL_RAZER_PLUGIN "${INSTALL_RAZER_PLUGIN}"
validate_toggle INSTALL_LIFX_PLUGIN "${INSTALL_LIFX_PLUGIN}"
validate_toggle INSTALL_PHILIPS_HUE_PLUGIN "${INSTALL_PHILIPS_HUE_PLUGIN}"
validate_toggle INSTALL_LINUX_LEDS_PLUGIN "${INSTALL_LINUX_LEDS_PLUGIN}"
validate_toggle INSTALL_WLED_PLUGIN "${INSTALL_WLED_PLUGIN}"
validate_toggle INSTALL_DBUS "${INSTALL_DBUS}"
validate_toggle INSTALL_DBUS_POLKIT "${INSTALL_DBUS_POLKIT}"
validate_toggle INSTALL_HTTP "${INSTALL_HTTP}"
if [ "${INSTALL_DBUS_POLKIT}" = "1" ] && [ "${INSTALL_DBUS}" != "1" ]; then
  printf '%s\n' "error: INSTALL_DBUS_POLKIT=1 requires INSTALL_DBUS=1" >&2
  exit 1
fi
if [ "${INSTALL_DBUS}" = "1" ]; then
  validate_abs_path DBUS_SYSTEM_SERVICES_DIR "${DBUS_SYSTEM_SERVICES_DIR}"
  validate_abs_path DBUS_SYSTEM_POLICY_DIR "${DBUS_SYSTEM_POLICY_DIR}"
fi
if [ "${INSTALL_HTTP}" = "1" ] && [ "${INIT_SYSTEM}" = "none" ]; then
  printf '%s\n' "error: INSTALL_HTTP=1 requires systemd or openrc integration" >&2
  exit 1
fi
if [ "${INSTALL_DBUS_POLKIT}" = "1" ]; then
  validate_abs_path POLKIT_ACTIONS_DIR "${POLKIT_ACTIONS_DIR}"
fi

BUILD_PROFILE="${BUILD_PROFILE:-release}"
BUILD_TARGET="${BUILD_TARGET:-}"

if [ "${BUILD_PROFILE}" = "release" ]; then
  TARGET_PROFILE_DIR="release"
elif [ "${BUILD_PROFILE}" = "debug" ] || [ "${BUILD_PROFILE}" = "dev" ]; then
  TARGET_PROFILE_DIR="debug"
else
  TARGET_PROFILE_DIR="${BUILD_PROFILE}"
fi

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "${repo_root}"

export LUMINATE_DEFAULT_CONFIG_PATH="${CONFIG_PATH}"
export LUMINATE_DEFAULT_SOCKET_PATH="${SOCKET_PATH}"
export LUMINATE_DEFAULT_STATE_PATH="${STATE_PATH}"
export LUMINATE_DEFAULT_HTTP_STATE_DIR="${HTTP_STATE_DIR}"
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
set -- "$@" \
  -p luminated \
  -p luminate-cli \
  -p libluminate
if [ "${INSTALL_DBUS}" = "1" ]; then
  set -- "$@" -p luminate-dbus --features luminate-dbus/dbus
fi
if [ "${INSTALL_HTTP}" = "1" ]; then
  set -- "$@" -p luminate-http
fi
if [ "${INSTALL_DEMO_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-demo-system
fi
if [ "${INSTALL_ALIENWARE_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-alienware
fi
if [ "${INSTALL_RAZER_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-razer
fi
if [ "${INSTALL_LIFX_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-lifx
fi
if [ "${INSTALL_PHILIPS_HUE_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-philips-hue
fi
if [ "${INSTALL_GOVEE_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-govee
fi
if [ "${INSTALL_LINUX_LEDS_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-linux-leds
fi
if [ "${INSTALL_WLED_PLUGIN}" = "1" ]; then
  set -- "$@" -p luminate-plugin-wled
fi
"$@"

if [ -n "${BUILD_TARGET}" ]; then
  target_dir="${repo_root}/target/${BUILD_TARGET}/${TARGET_PROFILE_DIR}"
else
  target_dir="${repo_root}/target/${TARGET_PROFILE_DIR}"
fi

install_file() {
  source_path=$1
  target_path=$2
  mode=$3

  install -D -m "${mode}" "${source_path}" "${DESTDIR}${target_path}"
}

install_symlink() {
  link_target=$1
  link_path=$2

  install -d -m 0755 "$(dirname -- "${DESTDIR}${link_path}")"
  ln -sf "${link_target}" "${DESTDIR}${link_path}"
}

# Must track the canonical `LUMINATE_C_ABI_VERSION` in
# crates/libluminate/src/c_abi.rs: the installed filename, the linker-recorded
# SONAME, and this symlink all have to agree for dynamic linking to resolve at
# runtime. The libluminate `c_abi_version` integration test checks this value.
#
# A bump here also means updating the literal `libluminate.so.<N>` asset
# paths and `[...provides]` entries (base table and the `lib32` variant) in
# crates/luminated/Cargo.toml -- cargo-generate-rpm/cargo-deb don't scan the
# packaged ELF for its SONAME the way real `rpmbuild` does.
LUMINATE_C_ABI_VERSION=28

# A private, exclusively-created staging directory for rendered templates.
# Using mktemp -d (rather than a predictable "$$"-based name reused across
# every render_template call) closes the symlink-following /
# clobber-and-race window a local attacker gets from a predictable path in a
# shared directory like /tmp.
install_tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/luminate-install.XXXXXX")
cleanup_install_tmp_dir() {
  rm -rf -- "${install_tmp_dir}"
}
trap cleanup_install_tmp_dir EXIT INT HUP TERM

# Literal (non-regex, non-sed) single-placeholder substitution: reads stdin,
# replaces every occurrence of $1 with $2, writes stdout. The value is passed
# through the environment and read back via ENVIRON (data, never interpolated
# into the awk program text and, unlike -v, not subject to backslash-escape
# processing), and the replacement is assembled with substr()/index() rather
# than gsub(), so a value containing sed/awk/regex metacharacters (or
# characters like `&`/`\` that gsub and -v both treat specially) is
# substituted as plain text. This is what keeps operator-supplied
# path/user/group overrides from being able to inject additional sed/awk
# commands or having their own content corrupted, unlike interpolating them
# straight into a sed program string.
literal_replace() {
  placeholder=$1
  value=$2
  LITERAL_REPLACE_KEY="${placeholder}" LITERAL_REPLACE_VAL="${value}" awk '
    BEGIN {
      key = ENVIRON["LITERAL_REPLACE_KEY"]
      val = ENVIRON["LITERAL_REPLACE_VAL"]
      klen = length(key)
    }
    {
      line = $0
      out = ""
      idx = index(line, key)
      while (idx > 0) {
        out = out substr(line, 1, idx - 1) val
        line = substr(line, idx + klen)
        idx = index(line, key)
      }
      print out line
    }
  '
}

render_template() {
  source_path=$1
  target_path=$2
  mode=$3
  tmp_path="${install_tmp_dir}/$(basename -- "${target_path}")"

  cat "${source_path}" |
    literal_replace "@BINDIR@" "${BINDIR}" |
    literal_replace "@CONFIG_DIR@" "${CONFIG_DIR}" |
    literal_replace "@CONFIG_PATH@" "${CONFIG_PATH}" |
    literal_replace "@EXAMPLE_CONFIG_PATH@" "${EXAMPLE_CONFIG_PATH}" |
    literal_replace "@LUMINATE_GROUP@" "${LUMINATE_GROUP}" |
    literal_replace "@LUMINATE_USER@" "${LUMINATE_USER}" |
    literal_replace "@LUMINATE_DBUS_USER@" "${LUMINATE_DBUS_USER}" |
    literal_replace "@LUMINATE_HTTP_USER@" "${LUMINATE_HTTP_USER}" |
    literal_replace "@HTTP_LISTEN@" "${HTTP_LISTEN}" |
    literal_replace "@HTTP_STATE_DIR@" "${HTTP_STATE_DIR}" |
    literal_replace "@PLUGIN_DIR_LOCAL@" "${PLUGIN_DIR_LOCAL}" |
    literal_replace "@PLUGIN_DIR_SYSTEM@" "${PLUGIN_DIR_SYSTEM}" |
    literal_replace "@PLUGIN_DIR_EXAMPLES@" "${PLUGIN_DIR_EXAMPLES}" |
    literal_replace "@RUNTIME_DIR@" "${RUNTIME_DIR}" |
    literal_replace "@SOCKET_PATH@" "${SOCKET_PATH}" |
    literal_replace "@STATE_DIR@" "${STATE_DIR}" |
    literal_replace "@STATE_PATH@" "${STATE_PATH}" \
      >"${tmp_path}"

  install_file "${tmp_path}" "${target_path}" "${mode}"
  rm -f -- "${tmp_path}"
}

install_file "${target_dir}/luminated" "${BINDIR}/luminated" 0755
install_file "${target_dir}/luminatectl" "${BINDIR}/luminatectl" 0755
install_file "${target_dir}/libluminate.so" "${SOLIBDIR}/libluminate.so.${LUMINATE_C_ABI_VERSION}" 0755
install_symlink "libluminate.so.${LUMINATE_C_ABI_VERSION}" "${SOLIBDIR}/libluminate.so"
install_file "${repo_root}/crates/libluminate/luminate.h" "${INCLUDEDIR}/luminate/luminate.h" 0644
if [ "${INSTALL_DBUS}" = "1" ]; then
  install_file "${target_dir}/luminate-dbus" "${BINDIR}/luminate-dbus" 0755
  render_template "${repo_root}/packaging/dbus/org.luminate.Luminate1.service.in" "${DBUS_SYSTEM_SERVICES_DIR}/org.luminate.Luminate1.service" 0644
  render_template "${repo_root}/packaging/dbus/org.luminate.Luminate1.conf.in" "${DBUS_SYSTEM_POLICY_DIR}/org.luminate.Luminate1.conf" 0644
fi
if [ "${INSTALL_HTTP}" = "1" ]; then
  install_file "${target_dir}/luminate-http" "${BINDIR}/luminate-http" 0755
fi
if [ "${INSTALL_DBUS_POLKIT}" = "1" ]; then
  install_file "${repo_root}/packaging/polkit/org.luminate.policy" "${POLKIT_ACTIONS_DIR}/org.luminate.policy" 0644
fi

# The demo plugin goes into the examples dir, which is not on the default
# autoload path, so a production install never exposes its fake devices.
if [ "${INSTALL_DEMO_PLUGIN}" = "1" ]; then
  install_file "${target_dir}/libluminate_plugin_demo_system.so" "${PLUGIN_DIR_EXAMPLES}/libluminate_plugin_demo_system.so" 0755
fi

if [ "${INSTALL_ALIENWARE_PLUGIN}" = "1" ]; then
  install_file "${target_dir}/libluminate_plugin_alienware.so" "${PLUGIN_DIR_SYSTEM}/libluminate_plugin_alienware.so" 0755
  render_template \
    "${repo_root}/plugins/luminate-plugin-alienware/packaging/udev/60-luminate-alienware.rules.in" \
    "${UDEV_RULES_DIR}/60-luminate-alienware.rules" \
    0644
fi

if [ "${INSTALL_RAZER_PLUGIN}" = "1" ]; then
  install_file "${target_dir}/libluminate_plugin_razer.so" "${PLUGIN_DIR_SYSTEM}/libluminate_plugin_razer.so" 0755
  render_template \
    "${repo_root}/plugins/luminate-plugin-razer/packaging/udev/60-luminate-razer.rules.in" \
    "${UDEV_RULES_DIR}/60-luminate-razer.rules" \
    0644
fi

if [ "${INSTALL_LIFX_PLUGIN}" = "1" ]; then
  install_file "${target_dir}/libluminate_plugin_lifx.so" "${PLUGIN_DIR_SYSTEM}/libluminate_plugin_lifx.so" 0755
fi

if [ "${INSTALL_PHILIPS_HUE_PLUGIN}" = "1" ]; then
  install_file "${target_dir}/libluminate_plugin_philips_hue.so" "${PLUGIN_DIR_SYSTEM}/libluminate_plugin_philips_hue.so" 0755
fi

if [ "${INSTALL_GOVEE_PLUGIN}" = "1" ]; then
  install_file "${target_dir}/libluminate_plugin_govee.so" "${PLUGIN_DIR_SYSTEM}/libluminate_plugin_govee.so" 0755
fi

if [ "${INSTALL_LINUX_LEDS_PLUGIN}" = "1" ]; then
  install_file "${target_dir}/libluminate_plugin_linux_leds.so" "${PLUGIN_DIR_SYSTEM}/libluminate_plugin_linux_leds.so" 0755
  render_template \
    "${repo_root}/plugins/luminate-plugin-linux-leds/packaging/udev/60-luminate-linux-leds.rules.in" \
    "${UDEV_RULES_DIR}/60-luminate-linux-leds.rules" \
    0644
fi

if [ "${INSTALL_WLED_PLUGIN}" = "1" ]; then
  install_file "${target_dir}/libluminate_plugin_wled.so" "${PLUGIN_DIR_SYSTEM}/libluminate_plugin_wled.so" 0755
fi

render_template "${repo_root}/packaging/luminated.toml.in" "${CONFIG_PATH}" 0644
if [ "${INSTALL_DEMO_PLUGIN}" = "1" ]; then
  render_template "${repo_root}/packaging/luminated-demo.toml.in" "${EXAMPLE_CONFIG_PATH}" 0644
fi

append_frontend_registration() {
  config_path=$1
  account=$2
  printf '\n[[authorization.frontends]]\nplatform = "unix"\nuser = "%s"\n' \
    "${account}" >>"${DESTDIR}${config_path}"
}

for rendered_config in "${CONFIG_PATH}" "${EXAMPLE_CONFIG_PATH}"; do
  if [ "${rendered_config}" = "${EXAMPLE_CONFIG_PATH}" ] && [ "${INSTALL_DEMO_PLUGIN}" != "1" ]; then
    continue
  fi
  if [ "${INSTALL_DBUS}" = "1" ]; then
    append_frontend_registration "${rendered_config}" "${LUMINATE_DBUS_USER}"
  fi
  if [ "${INSTALL_HTTP}" = "1" ]; then
    append_frontend_registration "${rendered_config}" "${LUMINATE_HTTP_USER}"
  fi
done

case "${INIT_SYSTEM}" in
systemd)
  render_template "${repo_root}/packaging/systemd/luminated.service.in" "${SYSTEMD_UNIT_DIR}/luminated.service" 0644
  render_template "${repo_root}/packaging/sysusers/luminate.conf.in" "${SYSUSERS_DIR}/luminate.conf" 0644
  render_template "${repo_root}/packaging/tmpfiles/luminate.conf.in" "${TMPFILES_DIR}/luminate.conf" 0644
  if [ "${INSTALL_DBUS}" = "1" ]; then
    render_template "${repo_root}/packaging/systemd/luminate-dbus.service.in" "${SYSTEMD_UNIT_DIR}/luminate-dbus.service" 0644
    render_template "${repo_root}/packaging/sysusers/luminate-dbus.conf.in" "${SYSUSERS_DIR}/luminate-dbus.conf" 0644
  fi
  if [ "${INSTALL_HTTP}" = "1" ]; then
    render_template "${repo_root}/packaging/systemd/luminate-http.service.in" "${SYSTEMD_UNIT_DIR}/luminate-http.service" 0644
    render_template "${repo_root}/packaging/sysusers/luminate-http.conf.in" "${SYSUSERS_DIR}/luminate-http.conf" 0644
    render_template "${repo_root}/packaging/tmpfiles/luminate-http.conf.in" "${TMPFILES_DIR}/luminate-http.conf" 0644
  fi
  ;;
openrc)
  render_template "${repo_root}/packaging/openrc/luminated.initd.in" "${OPENRC_INIT_DIR}/luminated" 0755
  render_template "${repo_root}/packaging/openrc/luminated.confd.in" "${OPENRC_CONF_DIR}/luminated" 0644
  if [ "${INSTALL_HTTP}" = "1" ]; then
    render_template "${repo_root}/packaging/openrc/luminate-http.initd.in" "${OPENRC_INIT_DIR}/luminate-http" 0755
    render_template "${repo_root}/packaging/openrc/luminate-http.confd.in" "${OPENRC_CONF_DIR}/luminate-http" 0644
  fi
  ;;
none) ;;
esac

printf '%s\n' "Installed Luminate into ${DESTDIR:-/} using build-time defaults:"
printf '  config: %s\n' "${CONFIG_PATH}"
printf '  socket: %s\n' "${SOCKET_PATH}"
printf '  state:  %s\n' "${STATE_PATH}"
printf '  plugin system dir: %s\n' "${PLUGIN_DIR_SYSTEM}"
printf '  demo plugin (not autoloaded): %s\n' "${PLUGIN_DIR_EXAMPLES}"
printf '  demo config (opt-in via LUMINATED_CONFIG): %s\n' "${EXAMPLE_CONFIG_PATH}"
printf '  init system integration: %s\n' "${INIT_SYSTEM}"
printf '  optional D-Bus bridge: %s\n' "${INSTALL_DBUS}"
printf '  optional HTTP front end: %s\n' "${INSTALL_HTTP}"
