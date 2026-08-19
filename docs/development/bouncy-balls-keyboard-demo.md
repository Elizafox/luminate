<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Bouncy balls on the M16 R2 keyboard

This Linux-only example runs up to ten rainbow-coloured balls across the
Alienware M16 R2 US-ANSI keyboard. It sends volatile full frames through
`libluminate`; it never asks the keyboard or daemon to persist a frame.

Build the example from the repository root:

```sh
cargo build --release -p libluminate --example bouncy_balls
```

Find the main keyboard input device. Stable `by-path` or `by-id` names are
preferable to an `eventN` number, which can change after rebooting:

```sh
ls -l /dev/input/by-path/*-event-kbd
```

On the M16 R2 used to develop the demo, the main keyboard is
`/dev/input/by-path/platform-i8042-serio-0-event-kbd`. Start five balls with:

```sh
LUMINATED_SOCKET_PATH=/run/luminated/luminated.sock \
  target/release/examples/bouncy_balls \
  --input /dev/input/by-path/platform-i8042-serio-0-event-kbd \
  --balls 5
```

`--balls` accepts 1 through 10 and defaults to 5. `--seed N` makes placement,
initial velocities, and key kicks repeatable. Set `LUMINATED_SOCKET_PATH` when
the daemon does not use Luminate's default system socket.

## Fullscreen input

The demo enters the terminal's alternate screen and exclusively grabs the
selected evdev device. Its keys will not reach the terminal, desktop, or other
applications until the demo exits. Press Escape to release the device, restore
the terminal, end the frame stream, and leave safely. After Escape is pressed,
the demo keeps the exclusive grab until Escape and every obstruction key have
been physically released. This prevents held-key repeat events from spilling
into the shell prompt during teardown.

Opening and grabbing `/dev/input/event*` normally requires membership in the
system's `input` group or an equivalent local ACL. Running the demo as root also
works, but granting narrowly scoped device access is preferable for routine
use.

Each held key becomes a circular obstruction in the simulation. Pressing a key
under a ball also gives that ball a new random direction at its normal speed.
Key-repeat events are ignored, and releasing the key removes the obstruction.
The firmware-handled Fn key does not emit an ordinary evdev key event. Some
media or vendor buttons may appear on a separate input device and are not
captured when only the main keyboard device is grabbed.

## Rendering and physics

The simulation uses a continuous 16-by-6 world with fixed 120 Hz physics. Balls
have equal mass, resolve pairwise elastic collisions, and reflect elastically
from walls and held-key obstructions. A key kick deliberately injects a new
direction, while preserving the ball's nominal speed.

Each ball receives an evenly spaced colour from the hue wheel. The scene is
first rasterized with soft edges onto a 64-by-24 RGB grid, then bilinearly
sampled at the photographed physical centre of each of the keyboard's 85 LEDs.
This intermediate grid shares the world's 16:6 aspect ratio and keeps motion
smooth as a ball moves between widely and unevenly spaced keys. Frames are sent
at 12 Hz to leave pacing headroom below the keyboard's 15 Hz limit.
