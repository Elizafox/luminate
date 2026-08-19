<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Razer profile import manifest

This manifest tracks the deliberate port from OpenRazer revision
`6820f9da169d354bc7e6e93a0aa8683a6bb75792`. It is review data, not generated
build input. A Razer VID alone never makes a device eligible for discovery.

## Imported profiles

| Luminate slug | VID:PID | Category | Transport | Matrix | Validation | OpenRazer sources |
| --- | --- | --- | --- | --- | --- | --- |
| `blackwidow-v4-pro` | `1532:028d` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 8×23 | `PartiallyValidated` | `driver/razerkbd_driver.c`, `driver/razerchromacommon.c`, `driver/razercommon.c`, `daemon/openrazer_daemon/hardware/keyboards.py` |
| `blackwidow-v4` | `1532:0287` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 8×23 | `Untested` | Same sources |
| `blackwidow-v4-75` | `1532:02a5` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 6×16 | `Untested` | Same sources |
| `blackwidow-v4-mini-hyperspeed-wired` | `1532:02b9` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 5×14 | `Untested` | Same sources |
| `blackwidow-v4-tenkeyless-hyperspeed-wired` | `1532:02d7` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 6×18 | `Untested` | Same sources |
| `blackwidow-v3-mini-hyperspeed-wired` | `1532:0258` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 5×16 | `Untested` | Same sources; OpenRGB detector and device table |
| `huntsman-mini-analog` | `1532:0282` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 5×15 | `Untested` | Same sources; OpenRGB detector and device table |
| `huntsman-v2-analog` | `1532:0266` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 9×22 combined keyboard and underglow | `Untested` | Same sources; OpenRGB MR !950 and device table |
| `huntsman-v2-tenkeyless` | `1532:026b` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 6×17 | `Untested` | Same sources; OpenRGB issue #2441 and corrected device table |
| `huntsman-v2` | `1532:026c` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 6×22 | `Untested` | Same sources; OpenRGB detector and device table |
| `huntsman-v3-pro` | `1532:02a6` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 6×22 | `Untested` | Same sources; OpenRGB detector and device table |
| `huntsman-v3-pro-tenkeyless` | `1532:02a7` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 6×19 | `Untested` | Same sources; OpenRGB issue #4418, MR !2627, and device table |
| `deathstalker-v2` | `1532:0295` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 6×22 | `Untested` | Same sources; OpenRGB issue #2904 and device table |
| `deathstalker-v2-pro-wired` | `1532:0292` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 6×22 | `Untested` | Same sources; OpenRGB detector and device table |
| `deathstalker-v2-pro-tenkeyless-wired` | `1532:0298` | Keyboard | Common 90-byte feature report, interface 3, transaction `0x1f`, 600 μs | 6×17 | `Untested` | Same sources; OpenRGB issue #3327 and device table |

The validated profile exposes only operations tested on the attached unit. See
the [hardware protocol specification](blackwidow-v4-pro-rgb-hid-protocol-spec.md)
for packet and visual evidence.

OpenRGB commits `bcfaa7e8a740a414b96cd65c397e3dfc` and
`02d0e16a72f9b007e16e60ec2a68852fb81242f0` are also used as temporary,
non-build research references. The latter supplies the newer Huntsman V3 Pro
TKL evidence. Their GPL-2.0-or-later device tables can
corroborate matrix layouts and help find contradictions in OpenRazer's broader
catalogue. They do not establish a safe HID interface, transport behaviour, or
hardware validation for another physical model.

OpenRazer represents the transaction byte as a three-bit device selector and a
five-bit request identifier. Consequently, its `0x1f` and OpenRGB's `0x3f` or
`0x9f` values all route to device 7; the upper bits are alternative request
tags echoed by the response. This is not a command-specific transport conflict,
so the imported wired profiles consistently use OpenRazer's `0x1f`.

## Deferred profile families

OpenRazer's declarations have been grouped by transport behaviour before
import. The following are deliberate safety deferrals rather than statements
that the devices lack lighting support.

| Family | Upstream evidence | Current blocker |
| --- | --- | --- |
| Other wired common-report keyboards | `razerkbd_driver.c`, `hardware/keyboards.py`, OpenRGB Razer tables | Remaining candidates still need exact interface evidence, bounded extended-matrix capabilities, and consistent topology data. |
| Wireless and receiver-routed keyboards | `razerkbd_driver.c`, `hardware/keyboards.py` | Receiver identity/routing, wired-versus-wireless PID pairing, transaction `0x9f`, and reconnect semantics need focused tests. |
| Classic and single-zone keyboards | `razerkbd_driver.c`, `razerchromacommon.c` | Standard-matrix/classic command capabilities and exact HID collection selectors are not yet represented by the imported extended-matrix profile. |
| Mice and mouse docks | `razermouse_driver.c`, `hardware/mice.py` | Mouse extended-matrix commands use a distinct class/command shape; input-safe collection selection and zone topology need separate fixtures. |
| Mats, stands, keypads, and accessories | `razeraccessory_driver.c`, `hardware/accessory.py` | Zero-target and accessory-specific zone semantics need declarative profiles plus exact interface evidence. |
| Headsets | `razeraccessory_driver.c`, `hardware/headsets.py` | Audio-composite collection selection and small zone maps have not been verified through `hidapi`. |
| ARGB controllers | `razeraccessory_driver.c`, `razercommon.c` | The 320-byte ARGB report and channel routing are a separate transport not implemented by the common report core. |
| Old-device report products | `razercommon.c` and per-category drivers | Legacy control-transfer value, size, and response behaviour require a separate bounded transport. |
| Laptops | `razerkbd_driver.c`, `hardware/laptops.py` | Internal composite-device selection and platform coexistence have not been tested; several models have product-specific matrix sizes and brightness paths. |

These blockers satisfy the current safety rule: no profile is imported until
it has an exact product ID, a positively selectable non-input control
interface, a bounded command family, and internally consistent topology data.
The `enable_untested_devices` setting defaults to `true`. Disabling it prevents
the provisional profiles from being opened or published without hiding
the `PartiallyValidated` BlackWidow V4 Pro.

## Refresh workflow

To refresh from OpenRazer:

1. Select and record a new upstream commit; never follow a moving branch in a
   build or test.
2. Diff the four driver sources and daemon hardware declarations named above,
   plus `hardware/mice.py`, `hardware/headsets.py`, and `hardware/laptops.py`.
3. Classify new and changed products into the transport families in this
   manifest. Record contradictions or incomplete declarations as blockers.
4. Review GPL provenance and copyright attribution for every substantially
   adapted command or table.
5. Add or change typed profiles and packet fixtures by hand. Do not overwrite
   hardware-validated topology, interface selectors, or validation status from
   upstream declarations.
6. Regenerate narrow packaging permissions from the reviewed imported IDs,
   then run registry, protocol, conformance, packaging, and relevant hardware
   checks.
7. Review profile removals, stable IDs, topology, and capability changes as
   compatibility-sensitive even while the plugin remains experimental.

The upstream checkout is research material only and must remain outside the
repository and all build inputs.
