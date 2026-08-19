# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

FROM rust:1.89-bookworm AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends libudev-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src/luminate
COPY . .

RUN DESTDIR=/opt/luminate-root \
    PREFIX=/usr \
    SYSCONFDIR=/etc \
    LOCALSTATEDIR=/var \
    RUNDIR=/run \
    INIT_SYSTEM=systemd \
    packaging/install-linux.sh

FROM debian:bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates libudev1 systemd systemd-sysv util-linux \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /opt/luminate-root /

RUN systemd-sysusers /usr/lib/sysusers.d/luminate.conf \
    && systemd-tmpfiles --create /usr/lib/tmpfiles.d/luminate.conf \
    && install -d /etc/systemd/system/luminated.service.d \
    && printf '%s\n' \
        '[Service]' \
        'Environment=LUMINATED_CONFIG=/usr/share/luminate/examples/luminated-demo.toml' \
        > /etc/systemd/system/luminated.service.d/smoke.conf \
    && systemctl enable luminated.service \
    && systemctl mask \
        systemd-logind.service \
        systemd-remount-fs.service \
        systemd-udevd.service \
        systemd-udevd-control.socket \
        systemd-udevd-kernel.socket

STOPSIGNAL SIGRTMIN+3
CMD ["/sbin/init"]
