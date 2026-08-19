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
    packaging/install-linux.sh

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libudev1 util-linux \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /opt/luminate-root /
COPY packaging/container-entrypoint.sh /usr/bin/luminate-container-entrypoint

RUN groupadd --system luminate \
    && useradd --system --home-dir /var/lib/luminated --gid luminate luminated \
    && chmod 0755 /usr/bin/luminate-container-entrypoint \
    && install -d -m 0750 -o luminated -g luminate /run/luminated \
    && install -d -m 0750 -o luminated -g luminate /var/lib/luminated

ENTRYPOINT ["/usr/bin/luminate-container-entrypoint"]
CMD ["/usr/bin/luminated"]
