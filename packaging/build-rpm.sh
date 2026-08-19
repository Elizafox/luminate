#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

# Build an .rpm using cargo-generate-rpm, from packaging/build-stage.sh's
# rendered tree. Requires cargo-generate-rpm (cargo install cargo-generate-rpm).
#
# cargo-generate-rpm's -p/--package flag is a filesystem path to the crate
# directory, not a package-name lookup via `cargo metadata` (despite the
# --help text saying "Name of a crate in the workspace") - so this passes
# the crate's relative path, not its Cargo.toml `name`.

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
BUILD_TARGET="${BUILD_TARGET:-}"
RPM_ARCH="${RPM_ARCH:-}"
INSTALL_DBUS="${INSTALL_DBUS:-0}"
INSTALL_DBUS_POLKIT="${INSTALL_DBUS_POLKIT:-0}"

if { [ -n "${BUILD_TARGET}" ] && [ -z "${RPM_ARCH}" ]; } \
  || { [ -z "${BUILD_TARGET}" ] && [ -n "${RPM_ARCH}" ]; }; then
  printf '%s\n' "BUILD_TARGET and RPM_ARCH must be set together for a cross-built RPM" >&2
  exit 1
fi

# Fedora keeps ELF shared objects under %_libdir, which is /usr/lib64 on
# 64-bit multilib arches (x86_64, aarch64, ...) but plain /usr/lib on 32-bit
# arches (i686) that have no /lib vs /lib64 split. `rpm --eval` reports the
# arch-correct value for the host building the package; SOLIBDIR must match
# it or the shared library ends up somewhere rpm's own dependency scanner
# doesn't look, breaking `libluminate.so.N()(64bit)` Provides/Requires.
case "${RPM_ARCH}" in
  "")
    solibdir=$(rpm --eval '%{_libdir}')
    ;;
  i586|i686)
    solibdir=/usr/lib
    PKG_CONFIG_ALLOW_CROSS="${PKG_CONFIG_ALLOW_CROSS:-1}"
    PKG_CONFIG_LIBDIR="${PKG_CONFIG_LIBDIR:-/usr/lib/pkgconfig:/usr/share/pkgconfig}"
    export PKG_CONFIG_ALLOW_CROSS PKG_CONFIG_LIBDIR
    if [ "${RPM_ARCH}" = "i586" ]; then
      CC_i586_unknown_linux_gnu="${CC_i586_unknown_linux_gnu:-/usr/bin/gcc}"
      export CC_i586_unknown_linux_gnu
    else
      CC_i686_unknown_linux_gnu="${CC_i686_unknown_linux_gnu:-/usr/bin/gcc}"
      export CC_i686_unknown_linux_gnu
    fi
    ;;
  *)
    printf 'unsupported cross-RPM architecture: %s\n' "${RPM_ARCH}" >&2
    exit 1
    ;;
esac

SOLIBDIR="${solibdir}" "${repo_root}/packaging/build-stage.sh"
cd "${repo_root}"

set -- cargo generate-rpm -p crates/luminated --target-dir "${repo_root}/target"
if [ -n "${RPM_ARCH}" ]; then
  set -- "$@" --arch "${RPM_ARCH}" --target "${BUILD_TARGET}"
fi
if [ "${solibdir}" != "/usr/lib64" ]; then
  # crates/luminated/Cargo.toml's base [package.metadata.generate-rpm]
  # assets/provides assume /usr/lib64; select the lib32 variant for arches
  # (i686, armv7hl, ...) where %_libdir is plain /usr/lib instead.
  set -- "$@" --variant=lib32
fi
if [ "${INSTALL_DBUS_POLKIT}" = "1" ]; then
  # Metadata overlays replace the complete table rather than merging nested
  # keys, so repeat the base requirements alongside both optional services.
  set -- "$@" --set-metadata \
    'requires = { systemd-udev = "*", systemd = "*", dbus = "*", polkit = "*" }'
elif [ "${INSTALL_DBUS}" = "1" ]; then
  set -- "$@" --set-metadata \
    'requires = { systemd-udev = "*", systemd = "*", dbus = "*" }'
fi
"$@"
