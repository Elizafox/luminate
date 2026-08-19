<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Packaging Luminate

Luminate selects its packaged layout at build time. The packaging script
passes those paths into the Rust build through
`LUMINATE_DEFAULT_*` variables, then renders the installed config and system
integration files with the same values.

Runtime path overrides still exist for local development, but packaged builds
should not rely on them.

## Default packaged layout on Linux

`packaging/install-linux.sh` defaults to:

- binaries: `/usr/bin/luminated`, `/usr/bin/luminatectl`
- C library: `/usr/lib/libluminate.so.28` with a `/usr/lib/libluminate.so` dev
  symlink (see `SOLIBDIR` below for the RPM/`/usr/lib64` case)
- C header: `/usr/include/luminate/luminate.h`
- plugins: `/usr/lib/luminate/plugins`
- installed plugins: Alienware, LIFX, Philips Hue, Govee, Linux LED-class, and
  WLED
- demo example config (opt-in via `LUMINATED_CONFIG`, not on the autoload
  path): `/usr/share/luminate/examples/luminated-demo.toml` — arch-independent
  app data under `/usr/share`, not `/usr/lib`, and deliberately not under
  `/usr/share/doc/luminate/` either, since it's live config the daemon reads
  directly rather than passive documentation (see the native packages section
  below for why that distinction matters)
- config: `/etc/luminate/luminated.toml`
- state: `/var/lib/luminated/state.json`
- managed config: `/var/lib/luminated/managed.toml`
- socket: `/run/luminated/luminated.sock`
- daemon user: `luminated`
- socket/device group: `luminate`

The optional HTTP front end adds `/usr/bin/luminate-http`, a dedicated
`luminate-http` service account, and private state under
`/var/lib/luminate-http`. It listens on `127.0.0.1:8080` by default. See
[`http-api.md`](http-api.md) for its security model and first-token procedure.
Keep that loopback default for a local reverse proxy. A direct LAN listener
must add the native TLS certificate and private-key options through a local
service override; packaging deliberately does not generate or select
site-specific certificate material.

The installed `luminated.toml` is administrator-owned bootstrap policy. The
installer renders it with mode `0644`, and the daemon never rewrites it. Its
default `plugin_management.activation = "host-attached"` activates plugins
which declare only known non-network buses. Network-capable plugins remain
installed and inspectable, but inactive until an administrator explicitly
selects them through global or managed policy.

The daemon writes machine-wide desired state to `managed.toml`, beside the
effective state file unless `[management].managed_config_path` selects a
different path. A missing managed file means revision zero and no overrides.
On the first managed change, the daemon creates or replaces it atomically with
mode `0600`. State, managed configuration, bearer-token, and persisted-policy
files must remain owned by the daemon identity and inaccessible to group and
other identities. Startup rejects permissive files, symlinks, directories, and
non-regular objects rather than trusting or blocking on them. Their containing
service directory may remain `0750`: the daemon must own it, and no other
identity may write it. An existing unreadable or invalid managed file prevents
startup; operators should not treat deleting it as ordinary error recovery
because it also discards the desired plugin and daemon-preference overrides.

## macOS source installation

`packaging/install-macos.sh` builds and installs the daemon, CLI, and portable
plugins using the machine-wide paths selected by the macOS port:

- binaries: `/usr/local/bin/luminated`, `/usr/local/bin/luminatectl`
- autoloaded plugins: `/usr/local/lib/luminate/plugins`
- example plugins: `/usr/local/lib/luminate/plugins-examples`
- config: `/Library/Application Support/Luminate/luminated.toml`
- socket: `/var/run/luminated/luminated.sock`
- state: `/Library/Application Support/Luminate/state/state.json`
- managed config:
  `/Library/Application Support/Luminate/state/managed.toml`
- logs: `/Library/Logs/luminated/luminated.log`

It is a source installer, not a signed or notarized `.pkg`. Run it as the
ordinary user who owns the checkout; it builds without privilege and invokes
`sudo` only while copying into system paths and managing the LaunchDaemon:

```sh
packaging/install-macos.sh
```

The default install registers the system LaunchDaemon but does not start it
immediately. It will start at the next boot because the plist uses `RunAtLoad`.
Opt into an immediate start when live hardware discovery is intended:

```sh
START_SERVICE=1 packaging/install-macos.sh
```

LIFX, Philips Hue, Govee, WLED, and the demo-system plugin are built by default.
Disable individual plugins with the corresponding `0`/`1` toggle:

```sh
INSTALL_LIFX_PLUGIN=0 \
INSTALL_PHILIPS_HUE_PLUGIN=0 \
INSTALL_GOVEE_PLUGIN=0 \
INSTALL_WLED_PLUGIN=0 \
INSTALL_DEMO_PLUGIN=0 \
START_SERVICE=1 \
packaging/install-macos.sh
```

The network plugins are installed in the production plugin directory, so they
can be inspected and enabled without changing their binary paths. The packaged
`host-attached` activation mode does not activate them by default because they
declare a network bus. The demo-system plugin is installed outside the
production plugin directory so fake devices do not appear in an ordinary
catalogue or topology. A pre-existing `luminated.toml` remains administrator
policy and is not replaced by the installer.

For packaging tests, an absolute `DESTDIR` stages files without using `sudo`,
creating the service account, writing the launchd plist, or invoking
`launchctl`:

```sh
DESTDIR=/path/to/pkgroot packaging/install-macos.sh
```

The installed daemon owns service lifecycle and account validation:

```sh
sudo /usr/local/bin/luminated service status
sudo /usr/local/bin/luminated service stop
sudo /usr/local/bin/luminated service start
sudo /usr/local/bin/luminated service uninstall
```

Ordinary uninstall retains the `_luminated` account, installed files, saved
state, and logs. `service uninstall --purge-account` additionally removes the
dedicated account only when it still exactly matches the installer-owned
identity. Artefact and data deletion remain deliberate administrator actions;
the source installer does not provide a destructive all-in-one removal path.

## Desktop components

The optional desktop component additionally installs `/usr/bin/luminate-dbus`,
system-bus activation/policy files, and (with systemd selected) a dedicated
`luminate-dbus.service`. It is selected with `INSTALL_DBUS=1`; the default is
off so base, minimal, and Alpine/OpenRC packages remain D-Bus-free. Set
`INSTALL_DBUS_POLKIT=1` to include the optional `org.luminate.control` Polkit
action; the bridge must also be started with `--polkit` to consult it.

The bridge runs under its own dedicated service account, `luminate-dbus` by
default (override with `LUMINATE_DBUS_USER`), rather than root or the daemon
account. It joins the client group so it can reach the control socket, and it
is a distinct Unix identity from every other client, so a daemon
daemon authorization policy can grant it an explicit, scoped ceiling (see
"D-Bus and Polkit" in
[`client-authorization.md`](architecture/client-authorization.md)) instead of
seeing a generic privileged identity. The bridge's own group/Polkit decision
and the daemon's independent authorization of the bridge's Unix identity are
both enforced; the effective result is their intersection.

The installed service exports the versioned contract documented in the
[D-Bus consumer guide](dbus.md). Package smoke tests verify installation and
activation artefacts; semantic method and signal coverage runs on an isolated
bus in the crate integration suite.

When the D-Bus or HTTP component is selected, the rendered daemon
configuration registers its service-account name as a front-end actor. The
daemon resolves that name after sysusers or the package manager has created
the account and refuses startup if it cannot be resolved. This keeps DEB/RPM
artefacts independent of installation-specific numeric UIDs. Overriding
`LUMINATE_DBUS_USER` or `LUMINATE_HTTP_USER` updates the service integration
and matching registration together.

Package managers preserve locally modified daemon configuration during an
upgrade. If an upgrade leaves the new configuration as a `.rpmnew` or asks to
retain the installed DEB conffile, merge the corresponding front-end
registrations into the active `luminated.toml` before using token or
attestation authentication through those services.

Native packages built with `INSTALL_DBUS=1` declare the system D-Bus runtime
as a package dependency (`dbus` on deb and rpm). Adding
`INSTALL_DBUS_POLKIT=1` also declares the policy service (`polkitd` on deb,
`polkit` on rpm). The base package retains neither dependency when the desktop
bridge is omitted.

## HTTP component

Set `INSTALL_HTTP=1` to build and install the pre-release HTTP API and the
selected init-system integration. It is off by default, so installing Luminate
does not unexpectedly add a TCP listener. Override its dedicated account,
private state path, or loopback listener with `LUMINATE_HTTP_USER`,
`HTTP_STATE_DIR`, and `HTTP_LISTEN`. Non-loopback values are rejected by the
installer; expose the service through a trusted TLS reverse proxy instead.

Systemd installs `luminate-http.service` plus sysusers and tmpfiles entries.
OpenRC installs matching init and conf files. Both run the process as the
dedicated HTTP identity in the `luminate` client group, with a `0077` umask.
Neither integration creates a bearer token or enables the service
automatically; token provisioning is an explicit administrator action.

The systemd unit runs the daemon as `luminated:luminate`. Its privileged
`ExecStartPre=` lines create the runtime/state directories, then systemd drops
privileges for `luminated` itself. Production udev rules grant matching hidraw
nodes directly to the daemon account at mode `0600`; membership in the
`luminate` client group does not grant raw hardware access.

The runtime directory is `0750`: clients need traversal permission plus the
socket's `0660` mode, but must not be able to unlink and replace the daemon
socket. At startup the daemon rejects a socket parent not owned by its effective
user or writable by group/others. During shutdown it only unlinks the exact
socket inode it created.

The state directory uses the same owner-controlled `0750` posture. Group
traversal of the directory does not widen access to its daemon-owned `0600`
authority files. Administrators remain responsible for ensuring every ancestor
of configured runtime and state paths is not writable by an untrusted user;
the daemon validates the final directory and opens authority files without
following their final path component.

`systemctl stop luminated` (systemd's default stop signal is `SIGTERM`, which
the unit doesn't override) triggers a graceful shutdown: the daemon stops
accepting new connections, gives in-flight ones up to 10 seconds to finish,
unlinks the socket file, then exits. `SIGHUP` reloads every currently loaded
plugin at its existing path and configuration; it does not re-read
`luminated`'s own configuration file for added or removed plugin entries.

The OpenRC service uses the same account, directory modes, compiled paths, and
`SIGTERM` shutdown contract. Its `start_pre` creates the runtime and state
directories with `checkpath`, then OpenRC starts the daemon directly as
`luminated:luminate`; no systemd compatibility layer is involved.

## Build-time configuration

Native build dependencies:

- Rust 1.89 or newer matching the workspace `rust-version`
- `pkg-config`
- `libudev` development files (`libudev-dev` on Debian/Ubuntu,
  `systemd-devel` on Fedora/RHEL-style systems)

Override paths by setting environment variables when invoking the installer:

```sh
DESTDIR=/tmp/pkgroot \
PREFIX=/usr \
SYSCONFDIR=/etc \
LOCALSTATEDIR=/var \
RUNDIR=/run \
LUMINATE_USER=luminated \
LUMINATE_GROUP=luminate \
packaging/install-linux.sh
```

`LIBDIR` (default `${PREFIX}/lib`) covers arch-independent data: systemd
units, sysusers/tmpfiles/udev config, and the demo example config. ELF shared
objects (`libluminate.so*`, plugin `.so`s) instead follow `SOLIBDIR`, which
defaults to `LIBDIR` but should be overridden to `/usr/lib64` on Fedora and
other 64-bit multilib distros, where `%_libdir` is not `/usr/lib`:

```sh
SOLIBDIR=/usr/lib64 packaging/install-linux.sh
```

`packaging/build-rpm.sh` does this automatically, deriving the right value
from `rpm --eval '%{_libdir}'` (which also correctly stays `/usr/lib` on
32-bit arches like i686/armv7hl) rather than hardcoding `/usr/lib64`.

More specific overrides are also available:

- `CONFIG_DIR`
- `CONFIG_PATH`
- `RUNTIME_DIR`
- `SOCKET_PATH`
- `STATE_DIR`
- `STATE_PATH`
- `HTTP_STATE_DIR`
- `HTTP_LISTEN`
- `LUMINATE_HTTP_USER`
- `SOLIBDIR`
- `PLUGIN_DIR_LOCAL`
- `PLUGIN_DIR_SYSTEM`
- `SYSTEMD_UNIT_DIR`
- `SYSUSERS_DIR`
- `TMPFILES_DIR`
- `UDEV_RULES_DIR`
- `OPENRC_INIT_DIR`
- `OPENRC_CONF_DIR`
- `INIT_SYSTEM` (`systemd`, `openrc`, or `none`)
- `INSTALL_DEMO_PLUGIN` (`0` or `1`)
- `INSTALL_ALIENWARE_PLUGIN` (`0` or `1`)
- `INSTALL_LIFX_PLUGIN` (`0` or `1`)
- `INSTALL_PHILIPS_HUE_PLUGIN` (`0` or `1`)
- `INSTALL_LINUX_LEDS_PLUGIN` (`0` or `1`)
- `INSTALL_WLED_PLUGIN` (`0` or `1`)
- `INSTALL_DBUS` (`0` or `1`, default `0`)
- `INSTALL_DBUS_POLKIT` (`0` or `1`, default `0`; requires `INSTALL_DBUS=1`)
- `LUMINATE_DBUS_USER` (default `luminate-dbus`)
- `BUILD_PROFILE`

Every rendered path override must be absolute and use only ASCII letters,
digits, `/`, `.`, `_`, `+`, `~`, or `-`. This conservative shared character
set is literal in TOML, systemd, sysusers, and tmpfiles syntax; paths containing
spaces, quotes, backslashes, `%` specifiers, or other grammar metacharacters are
rejected before the build begins.

The same values are baked into the compiled defaults via:

- `LUMINATE_DEFAULT_CONFIG_PATH`
- `LUMINATE_DEFAULT_SOCKET_PATH`
- `LUMINATE_DEFAULT_STATE_PATH`
- `LUMINATE_DEFAULT_HTTP_STATE_DIR`
- `LUMINATE_DEFAULT_PLUGIN_DIR_LOCAL`
- `LUMINATE_DEFAULT_PLUGIN_DIR_SYSTEM`

Do not patch `luminate-platform::default_path` for distro-specific layouts; use the install
script or equivalent build-system environment instead.

`install-linux.sh` defaults to systemd integration for the existing deb/rpm path.
Selecting OpenRC installs only `/etc/init.d/luminated` and
`/etc/conf.d/luminated`; selecting `none` installs no service-manager files.
Plugin toggles also control which Cargo packages are built, so an Alpine/LIFX
package does not need the Alienware plugin's eudev development or runtime
dependency.

## Hardware permissions

`plugins/luminate-plugin-alienware/packaging/udev/60-luminate-alienware.rules.in`
owns the Alienware plugin's hardware rules and currently grants access for:

- Alienware/Darfon keyboard RGB controller: `0d62:d2b1`
- Dell/Alienware AW-ELC RGB controller: `187c:0550`
- Alienware AW-ELC RGB controller: `187c:0551`

The `187c:0550` rule grants the daemon account access to the HID node, but the
plugin still requires an exact known platform ID and expected zone count before
publishing writable topology. A USB ID match alone is not a support claim.

Hardware plugins should keep narrowly matched rule templates beside their own
source under `packaging/udev/`; the packaging installer renders and installs a
fragment only when it installs that plugin artifact. Do not add
`TAG+="uaccess"` or whole-USB-device access for interactive convenience. A
plugin that requires libusb needs a separately justified,
interface-scoped rule rather than a blanket USB grant.

## Smoke testing

Use the host-side packaged-layout smoke test when you want a fast check without
a container runtime:

```sh
packaging/smoke-test.sh
```

It installs into `/tmp/luminate-smoke-root` by default, bakes all daemon/client
defaults to that tree, starts the installed daemon, checks `luminatectl version`,
checks that the installed demo plugin appears in `luminatectl list --json`, and
sends one mutation through the installed client.

Use the Podman container smoke test when you want to validate the packaged image:

```sh
packaging/container-smoke-test.sh
```

It builds `luminate-packaging-test`, starts a detached daemon container with
temporary bind-mounted runtime/state directories, then uses `podman exec` to run
the installed `luminatectl` client inside the same container:

- `luminatectl version`
- `luminatectl list --json`
- one demo static `set-effect` mutation

The script uses Podman by default. Override with:

```sh
CONTAINER_ENGINE=docker packaging/container-smoke-test.sh
```

The ordinary container test exercises the installed daemon through the image's
minimal privilege-dropping entrypoint; it does not claim to run systemd. Use the
separate systemd lifecycle test for the packaged unit itself:

```sh
packaging/systemd-smoke-test.sh
```

That image boots systemd as PID 1, starts `luminated.service` as the packaged
daemon account, performs a mutation, restarts the service with `systemctl`,
verifies restored intended state, and stops the unit while checking socket
cleanup. It requires a privileged test container so systemd can manage its
namespaces and cgroup.

For manual image testing with Podman:

```sh
podman build -f packaging/Dockerfile -t luminate-packaging-test .

podman run --rm \
  -v /tmp/luminate-container-run:/run/luminated:Z \
  -v /tmp/luminate-container-state:/var/lib/luminated:Z \
  luminate-packaging-test
```

The container entrypoint intentionally starts as root, prepares `/run/luminated`
and `/var/lib/luminated`, then drops to `luminated:luminate` before executing
the daemon. This mirrors the packaged systemd unit's privileged `ExecStartPre=`
directory setup and avoids bind-mounted runtime/state directories failing due
to host ownership.

For real HID hardware, container runs still need explicit device access from the
host, for example `--device /dev/hidrawN` plus matching permissions.

## Alpine, musl, and OpenRC

The Alpine path builds natively on Alpine 3.22 with Rust 1.89 and packages the
rendered tree as real `.apk` artifacts. Run its end-to-end check with:

```sh
packaging/alpine-smoke-test.sh
```

This test deliberately builds with dynamic musl via
`RUSTFLAGS="-C target-feature=-crt-static"`. Rust's musl target otherwise
defaults to a static C runtime, a mode that cannot emit the `cdylib` artifacts
used by `libluminate.so` and native plugins. The image build rejects any
`libc.so.6` or `ld-linux` dependency and verifies musl interpreters/NEEDED
entries for the daemon, CLI, C library, portable network plugins including
Philips Hue, and the demo plugin.

The smoke runtime installs the resulting `luminate` and `luminate-openrc` APKs,
then uses `rc-service` to start, stop, and restart the unprivileged daemon. It
checks native plugin loading, CLI access, a mutation, graceful socket cleanup,
and restoration of persisted intended state after restart. The Alienware
plugin is disabled for this LIFX/demo package, demonstrating that eudev remains
an optional hardware-backend dependency rather than a core Alpine dependency.

`packaging/alpine/APKBUILD` follows the same staged-tree model as the deb/rpm
prototype. The OpenRC files are split into the conventional
`luminate-openrc` subpackage; package installation creates the daemon account,
but does not enable or start the service. An administrator enables it with:

```sh
rc-update add luminated default
rc-service luminated start
```

## Native deb/rpm packages

`cargo-deb` and `cargo-generate-rpm` build real `.deb`/`.rpm` packages
directly from the same rendered tree `packaging/install-linux.sh` produces, rather
than duplicating path/template rendering as separate packaging metadata.
This is a lightweight prototype, not full distro-native packaging (no deb
changelog, no dedicated dev/lib subpackage split, no rpm spec written by
hand). That's deliberately deferred until there's an actual versioned
release to hang it on; carrying distribution-specific release machinery before
there is a release would create maintenance work without testing a real upgrade
or publication path.

Install the tools once:

```sh
cargo install cargo-deb cargo-generate-rpm
```

`packaging/build-rpm.sh` additionally needs the `rpm` binary on `PATH` (just
for `rpm --eval '%{_libdir}'`; it doesn't shell out to `rpmbuild`). The RPM
smoke test also uses `rpm2cpio`, `cpio`, and `readelf` to inspect the generated
artefact.

Then build either package:

```sh
packaging/build-deb.sh
packaging/build-rpm.sh
```

To cross-build a 32-bit x86 RPM on a Fedora multilib host, install the
corresponding Rust target plus `glibc-devel.i686` and `systemd-devel.i686`,
then set both target names:

```sh
BUILD_TARGET=i686-unknown-linux-gnu RPM_ARCH=i686 packaging/build-rpm.sh
```

`i586-unknown-linux-gnu`/`RPM_ARCH=i586` is supported in the same way.

Both scripts run `packaging/build-stage.sh` first, which installs into
`target/pkgroot` (a `DESTDIR` under `packaging/install-linux.sh`'s own defaults),
then package that tree's files as pre-built assets
(`cargo deb --no-build` / `cargo generate-rpm`). Output lands at
`target/debian/luminate_<version>_<arch>.deb` and
`target/generate-rpm/luminate-<version>.<arch>.rpm`.

`build-rpm.sh` stages `SOLIBDIR` (see above) at whatever `rpm --eval
'%{_libdir}'` reports for the build host and passes `--variant=lib32` to
`cargo generate-rpm` on arches where that's plain `/usr/lib` rather than
`/usr/lib64`, selecting the matching
`[package.metadata.generate-rpm.variants.lib32]` table in
`crates/luminated/Cargo.toml`. `build-deb.sh` always uses plain `/usr/lib`;
Debian doesn't split `/usr/lib` vs `/usr/lib64`.

The RPM smoke test also unpacks the generated package before starting its
container. It checks the packaged library filename, `DT_SONAME`, generated C
header, and RPM Provides/Requires against the canonical C ABI version. Set
`VERIFY_RPM_ONLY=1` to run these artefact checks without starting the
container. Native checks follow the host's `%_libdir`; cross-target checks use
`BUILD_TARGET`/`RPM_ARCH` to select the target-specific artefact, library
directory, and RPM capability form. The integration test separately covers
both 64-bit and `lib32` manifest variants on every build.

Both packages:

- install the daemon/CLI binaries, `libluminate.so.28` + the `libluminate.so`
  dev symlink + header, both plugin `.so`s, the systemd unit,
  sysusers/tmpfiles/udev config, the demo example config, `LICENSE-GPLv3` +
  `LICENSE-LGPLv3` + `LICENSE-CC-BY-SA-4.0` under
  `/usr/share/luminate/licenses/`, and the rendered default `luminated.toml`
  (marked as a config file so local edits survive upgrades)
- declare `libudev1` (deb) / `systemd-udev` (rpm) as a dependency; rpm's
  `libudev.so.1` shared-library requirement is auto-detected from the built
  binaries
- run `systemd-sysusers`, `systemd-tmpfiles --create`, and a udev rules
  reload in a post-install script, so the daemon user/group and runtime
  directories exist immediately after install. They do not enable or
  start `luminated.service`; that's left to the admin
- stop `luminated.service` on removal via a pre-uninstall/prerm script

Both packages ship `libluminate.so.28` as its own asset, not just the
`libluminate.so` dev symlink: `cargo-deb`/`cargo-generate-rpm` don't scan
packaged ELFs for `DT_SONAME` the way `rpmbuild`'s `find-provides` does, so
without an explicit `libluminate.so.28` asset (and, on the RPM side, an
explicit `[package.metadata.generate-rpm.provides]` entry) the package could
end up requiring a SONAME it never actually provides. See
`LUMINATE_C_ABI_VERSION` in `packaging/install-linux.sh`.

Because none of this is derived automatically, bumping the C ABI version
(`LUMINATE_C_ABI_VERSION` in `crates/libluminate/src/c_abi.rs`) means updating
the packaging literals checked by the `c_abi_version` integration test: the
`LUMINATE_C_ABI_VERSION` assignment in
`packaging/install-linux.sh`, and the asset paths plus `[...provides]` entries in
`crates/luminated/Cargo.toml` (both the base table and the `lib32` RPM
variant). The integration test fails if one of these versions is missed,
before the mismatch reaches a package installation or dependency check.

`LICENSE-GPLv3`/`LICENSE-LGPLv3`/`LICENSE-CC-BY-SA-4.0` are shipped under
`/usr/share/luminate/licenses/`, not `/usr/share/doc/luminate/`, and on the RPM
side deliberately not marked `doc = true`. Both distros silently drop files from
minimal/container installs based on where or how a file is packaged, independent
of whether `dnf`/`apt` report success: `debian:bookworm-slim`'s default
`/etc/dpkg/dpkg.cfg.d/01_nodoc` does `path-exclude /usr/share/doc/*` (with
the literal filename `copyright` special-cased), and Fedora's container
images set `tsflags=nodocs`, which drops any file cargo-generate-rpm marks
`doc = true` (the closest thing it has to rpm's real `%license` flag)
regardless of path. A real license file shouldn't disappear either way, so
these are shipped as ordinary untyped files outside `/usr/share/doc/` on
both sides.

Packaging metadata lives in `crates/luminated/Cargo.toml` under
`[package.metadata.deb]` / `[package.metadata.generate-rpm]`, plus
`packaging/deb/postinst` and `packaging/deb/prerm` for the deb maintainer
scripts (`cargo-generate-rpm` takes its scriptlets inline in `Cargo.toml`
instead).

Not yet built/decided: AUR packaging (mentioned as a later idea, not
started), and whether to add a changelog / native versioning scheme once a
real release cycle exists.
