<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Luminate Keychron plugin

`luminate-plugin-keychron` is an experimental, offline host-side plugin for the
common RGB protocol in modern Keychron QMK firmware.

The current implementation is experimental and unvalidated. It selects only
Keychron vendor `0x3434` devices whose HID collection has the QMK Raw HID usage
page `0xff60` and usage `0x61`. It queries the Keychron protocol version,
feature bitmap, RGB protocol version, and LED count before publishing a generic
surface of `led-NNN` elements. These are firmware indices, not identified
physical keys, and have no guessed geometry.

Published devices carry `shape:keyboard` and `form:standalone`. The plugin does
not publish a layout tag because its generic discovery path does not establish
the keyboard's regional key arrangement.

Static colour and off update the firmware's runtime per-key HSV array in
batches of at most nine LEDs. They do not write EEPROM. `SaveCurrent` invokes
the separate firmware save command only after Luminate has established a
complete colour shadow for every reported LED in the current plugin session.
Brightness, firmware effects, reset, readback, and frame streaming remain
disabled until hardware validation supports them.

The plugin never opens ordinary keyboard, mouse, media, or consumer-control
collections. It installs no udev rule: granting access to every device from a
vendor would be too broad, and no exact product/interface pair has been
validated yet.

Until an exact product rule is added, hardware testing requires temporary
read/write access to the keyboard's `0xff60:0x61` hidraw node. Do not grant
access to every Keychron device or to its ordinary input collections. Launcher,
VIA, OpenRGB, and Luminate may contend for the same Raw HID collection; run
only one controller while testing.

Build and test the protocol scaffold with:

```sh
cargo build -p luminate-plugin-keychron
cargo test -p luminate-plugin-keychron
```

The next hardware-backed phase needs an exact model profile and stable identity,
validated LED mapping, volatile per-key writes, and a narrowly matched udev
rule. Persistence remains out of scope until a complete persistable state and
the firmware's EEPROM behaviour have been verified on that model.
