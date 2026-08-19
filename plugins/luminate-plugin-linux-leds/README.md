<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Linux LED class plugin

This plugin discovers keyboard backlights exposed by the kernel below
`/sys/class/leds`. It deliberately ignores other LED-class devices, including
status, storage-activity, radio, and power LEDs.

Supported names follow the kernel LED-class convention:

- `<device>:kbd_backlight` (including the common `<device>::kbd_backlight`)
- `<device>:<colour>:kbd_zoned_backlight-<zone>`

Brightness uses a normalized 0–100 Luminate scale and is converted to each
LED's `max_brightness`. RGB multicolour LEDs also support static colour through
`multi_index` and `multi_intensity`. Existing kernel triggers are never changed.
These sysfs devices are host-attached peripherals for authorization purposes.
Topology polling includes the stable kernel identity, grouping and zones,
route, maxima, RGB channel layout, and active trigger presentation so replacing
or reconfiguring a same-named entry refreshes the daemon topology.

For development and tests, `LUMINATE_LED_SYSFS_ROOT` may point at a fake
directory with the same per-LED attribute layout. Production deployments
normally leave it unset.

The daemon account needs write access to `brightness` and, for colour devices,
`multi_intensity`. The packaged udev rule grants the dedicated daemon user
mode-0600 ownership of only those attributes on standard keyboard-backlight
entries. Systems without udev must provide equivalent narrow permissions;
do not grant access to all of `/sys/class/leds`.
