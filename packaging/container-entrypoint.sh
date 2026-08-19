#!/usr/bin/env sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

# Container entrypoint equivalent of the packaged systemd ExecStartPre lines:
# prepare writable runtime/state directories as root, then drop privileges for
# the daemon itself.

install -d -m 0750 -o luminated -g luminate /run/luminated
install -d -m 0750 -o luminated -g luminate /var/lib/luminated

# Drop privileges the same way the systemd unit does (NoNewPrivileges plus no
# retained capabilities). Device access is granted through group ownership of
# the /dev nodes, not capabilities, so clearing the whole capability set is
# safe. --no-new-privs also blocks any setuid-based re-escalation from the
# daemon.
exec setpriv \
  --reuid=luminated \
  --regid=luminate \
  --init-groups \
  --no-new-privs \
  --inh-caps=-all \
  --ambient-caps=-all \
  --bounding-set=-all \
  -- \
  "$@"
