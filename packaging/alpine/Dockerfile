# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

ARG ALPINE_VERSION=3.22
ARG RUST_VERSION=1.89

FROM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS build

RUN apk add --no-cache alpine-sdk eudev-dev linux-headers pkgconf

WORKDIR /src/luminate
COPY . .

# Build natively on Alpine so the daemon, CLI, C library, and plugins all use
# musl. Keep the Alienware plugin in its own optional package boundary; Razer
# remains enabled and uses the eudev development files installed above.
# Rust's musl target defaults to crt-static, which cannot emit cdylibs. Dynamic
# musl is required here because libluminate and native plugins are first-class
# package artifacts, not optional casualties of static linking.
ENV RUSTFLAGS="-C target-feature=-crt-static"
RUN DESTDIR=/opt/luminate-root \
    PREFIX=/usr \
    SYSCONFDIR=/etc \
    LOCALSTATEDIR=/var \
    RUNDIR=/run \
    UDEV_RULES_DIR=/usr/lib/udev/rules.d \
    INIT_SYSTEM=openrc \
    INSTALL_ALIENWARE_PLUGIN=0 \
    packaging/install-linux.sh \
    && install -Dm644 LICENSE-GPLv3 \
      /opt/luminate-root/usr/share/licenses/luminate/LICENSE-GPLv3 \
    && install -Dm644 LICENSE-LGPLv3 \
      /opt/luminate-root/usr/share/licenses/luminate/LICENSE-LGPLv3 \
    && install -Dm644 LICENSE-CC-BY-SA-4.0 \
      /opt/luminate-root/usr/share/licenses/luminate/LICENSE-CC-BY-SA-4.0

# Reject accidental glibc linkage in every executable/shared object that is
# shipped by this image. scanelf reports both NEEDED libraries and interpreters.
RUN scanelf --recursive --needed --interp /opt/luminate-root \
      | tee /tmp/luminate-elf.txt \
    && ! grep -E 'libc\.so\.6|ld-linux' /tmp/luminate-elf.txt \
    && grep -F 'ld-musl-' /tmp/luminate-elf.txt \
    && test -x /opt/luminate-root/etc/init.d/luminated \
    && test ! -e /opt/luminate-root/usr/lib/systemd

# Turn the rendered tree into a real Alpine package. The runtime stage installs
# this APK, so package dependencies and account hooks are exercised too.
RUN adduser -D builder \
    && addgroup builder abuild \
    && install -d -o builder -g builder /home/builder/package /packages \
    && cp packaging/alpine/APKBUILD \
      packaging/alpine/luminate.pre-install \
      packaging/alpine/luminate-openrc.pre-deinstall \
      /home/builder/package/ \
    && chown -R builder:builder /home/builder /packages /opt/luminate-root \
    && su builder -c 'abuild-keygen -a -n' \
    && su builder -c \
      'cd /home/builder/package && REPODEST=/packages LUMINATE_STAGE_ROOT=/opt/luminate-root abuild -F rootpkg'

FROM alpine:${ALPINE_VERSION}

RUN apk add --no-cache ca-certificates libgcc openrc

COPY --from=build /packages /packages

RUN apk add --allow-untrusted /packages/*/*/luminate-*.apk \
    && rm -rf /packages \
    && rc-update add luminated default

CMD ["/bin/sh"]
