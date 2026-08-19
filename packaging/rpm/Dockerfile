# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

# Installs the actual generated .rpm artifact (not the raw staged tree) so
# the post_install/pre_uninstall scriptlets and package dependency
# resolution get exercised, not just the systemd unit content.

FROM fedora:44

COPY target/generate-rpm/*.rpm /tmp/luminate.rpm

RUN dnf install -y /tmp/luminate.rpm \
    && dnf clean all \
    && rm -f /tmp/luminate.rpm \
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
