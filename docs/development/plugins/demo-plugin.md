<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Run the demo plugin locally

The daemon defaults are aimed at a system install:

- config: `/etc/luminate/luminated.toml`
- socket: `/run/luminated.sock`
- plugin directories under `/usr/lib` and `/usr/local/lib`

Those defaults suit a system daemon but get in the way during local work.
`luminated` accepts explicit overrides, so the same binary can run as a regular
user or inside a container.

## Local development flow

Build the daemon and demo plugin:

```bash
cargo build -p luminated -p luminate-plugin-demo-system
```

Use the checked-in local demo config:

```bash
LUMINATED_CONFIG=/home/elizabeth/dev/luminate/docs/development/config/demo-plugin.local.toml \
cargo run -p luminated
```

That config uses a user-owned socket path and state file:

- `/tmp/luminated-demo/luminated.sock`
- `/tmp/luminated-demo/state.json`

and points directly at the built demo plugin `.so`:

- `/home/elizabeth/dev/luminate/target/debug/libluminate_plugin_demo_system.so`

With the daemon running, drive a device from another shell (point the CLI at
the same socket). The `demo-streaming-keypad` advertises a vendor hardware
effect, `demo-scene-show`, alongside the typed effects:

```bash
export LUMINATED_SOCKET_PATH=/tmp/luminated-demo/luminated.sock

# a typed effect (--effect matches a built-in effect name)
luminatectl set-effect --device demo-streaming-keypad --effect static --rgb '#0080ff'

# the vendor hardware effect, invoked by id with its declared arguments
# (no built-in effect named "demo-scene-show", so it falls back to hardware)
luminatectl set-effect --device demo-streaming-keypad \
  --effect demo-scene-show --choice nebula --speed 7
```

When `--effect`'s value isn't a recognized built-in effect name, it's treated
as a hardware effect id, building an `Effect::Hardware` invocation. Pass
`--kind hardware` or `--kind builtin` to force one interpretation instead of
relying on the name lookup. Hardware arguments come from `--rgb`/`--extra-rgb`
(colours), `--speed`, `--direction`, `--duration-ms`, `--brightness`, and
`--choice`. The daemon checks them against the advertised effect and rejects
unknown IDs, invalid choices, out-of-range values, and undeclared arguments.
Run `luminatectl list` and read the `hw-effects=` line to see each effect's IDs,
choices, and ranges.

## Useful overrides

You can also override paths directly:

- `LUMINATED_CONFIG`
  - path to an alternate TOML config file
- `LUMINATED_SOCKET_PATH`
  - force the daemon socket path regardless of config contents
- `LUMINATED_STATE_PATH`
  - force the persisted daemon state file path regardless of config contents

The daemon persists desired overlays after every successful mutation. Startup
and device reappearance then use an explicit reconciliation policy: `Restore`
replays the adopted baseline beneath desired overlays and standing rules,
`Adopt` reads exact hardware facets into a separate durable baseline, and
`Leave` does not write. Policy comes from user overrides, then the plugin's
recommendation, with `Leave` as the fallback. A write-through
`PersistenceRequirement::Required` target still avoids needless replay writes.

Two contrasting reconciliation devices live in sibling plugins, split out for
focus alongside the demo keyboard:

- `luminate-plugin-demo-bulb` publishes `demo-smart-bulb`
  (`PersistenceRequirement::Required`, models write-through flash persistence)
- `luminate-plugin-demo-ambient` publishes `demo-ambient-panel`, recommends
  `Adopt`, and implements exact bounded snapshot readback.

Build and load either plugin to compare persistence-driven write avoidance with
an actual hardware-preserving adoption read.

A third sibling, `luminate-plugin-demo-display`, publishes `demo-led-display`
and is the reference implementation for the
[zero-copy shared-memory fast path](../architecture/shm-frame-streaming-phase1.md).
It models a 16x16 matrix and implements both `FrameStreamingPlugin` and
`ShmFrameStreamingPlugin`. Load it, then drive
`BeginFrameStream`/`UploadFrame`/`EndFrameStream` against `demo-led-display`.
There is no CLI subcommand for this yet, so use `libluminate::Client` directly,
as in `crates/libluminate/examples/bouncy_balls.rs`. The daemon should log
`shared-memory frame stream active` with the negotiated `segment_bytes`,
confirming that the fast path activated rather than silently falling back to
the ordinary pipe.

Example:

```bash
LUMINATED_CONFIG=/home/elizabeth/dev/luminate/docs/development/config/demo-plugin.local.toml \
LUMINATED_SOCKET_PATH=/tmp/luminate-alt/luminated.sock \
cargo run -p luminated
```

## Development and system defaults

Packaged daemons should keep the system defaults. Overrides let the same build
run:

- as root on a real system
- locally during development
- inside a container or test harness

No special “dev mode” binary is required.
