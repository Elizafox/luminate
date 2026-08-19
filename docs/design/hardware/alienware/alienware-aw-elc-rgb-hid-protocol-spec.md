---
# SPDX-License-Identifier: CC-BY-SA-4.0
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford
title: Dell and Alienware AW-ELC RGB Hardware Notes
author:
  - Elizabeth Ashford
date: 2026-07-09
version: 1.0
description: Reverse-engineered and externally corroborated notes for Dell and Alienware AW-ELC RGB controllers.
license: CC BY 4.0
keywords:
  - alienware
  - reverse engineering
  - rgb
  - hid
  - protocol
lang: en
status: reverse-engineered hardware notes
---

# Dell and Alienware AW-ELC RGB Hardware Notes

These notes describe the Alienware AW-ELC RGB controller protocol used on the
Alienware m16 R2 for the trackpad ring, rear alien-head/lid-logo LED, and
power-button alien-head LED. The tested controller is VID:PID `0x187C:0x0551`,
exposed as a USB HID device with one interrupt-IN endpoint and no interrupt-OUT
endpoint.

The report family and m16 R2 behaviour are hardware-validated in Luminate.
Additional `187c:0550` platform identities and zone tables are corroborated by
OpenRGB master revision `5e6d627f519a791487e766aeee51eed7bf7129ef`
(recorded 2026-08-16), but have not yet been exercised by Luminate on their
named laptops. Corroborated facts are identified separately below; they must
not be read as hardware-validation claims.

The protocol described here is separate from the internal keyboard RGB protocol,
which uses fixed leading byte `0xCC` on VID:PID `0x0D62:0xD2B1`. AW-ELC reports
begin with wire byte `0x03` and use 33-byte HID feature reports.

## Reading the packet descriptions

- Bytes are written as lowercase hexadecimal.
- Examples include the hidapi userspace placeholder byte `00` unless explicitly
  labelled as wire reports.
- The wire report is 33 bytes. A hidapi buffer is 34 bytes: one placeholder
  byte followed by the 33-byte wire report.
- `03:<opcode>` denotes the AW-ELC wire command whose first byte is `0x03` and
  second byte is `<opcode>`.
- `R G B` denotes byte order red, green, blue.
- Unspecified bytes are zero-filled to the report length.
- For unknown fields, the examples retain values from known-good packets.

This is an informal account of reverse-engineered hardware, not a compatibility
standard or a claim about Dell's internal API stability. Terms such as "known",
"partly understood", and "unresolved" describe the evidence available from the
tested machine.

## 1. Device model

The controller has two programming paths.

| Target                              | Programming path                  | Persistence model                             |
| ----------------------------------- | --------------------------------- | --------------------------------------------- |
| Trackpad ring (`0x00`)              | Live animation envelope `03:21`   | Temporary/live unless saved as user animation |
| Rear alien head / lid logo (`0x02`) | Live animation envelope `03:21`   | Temporary/live unless saved as user animation |
| Power-button alien head (`0x04`)    | Power-state slot envelope `03:22` | Persistent fixed slots                        |

The live zones and the power-button zone are not interchangeable. Zone `0x04`
uses the `03:22` power-state path; the `03:21` live-animation path does not
program it.

AWCC often re-pushes all known zones when only one visible zone changes.
Matching that behaviour means keeping current state for all zones and rebuilding
full transactions as needed.

## 2. Transport and hidapi framing

| Property                 | Value                                         |
| ------------------------ | --------------------------------------------- |
| Controller               | `0x187C:0x0551` Dell/Alienware AW-ELC         |
| m16 R2 platform ID       | `0x1102`                                      |
| Interface                | USB HID                                       |
| Host-to-device channel   | `SET_REPORT` Feature transfer over endpoint 0 |
| Device-to-host read-back | `GET_REPORT` Feature transfer                 |
| Interrupt endpoint       | IN `0x81`; no interrupt-OUT                   |
| Wire report length       | 33 bytes (`wMaxPacketSize = 0x21`)            |
| Fixed first wire byte    | `0x03`                                        |

The HID report descriptor does not define numbered reports. hidapi still uses
`buf[0]` as its report-ID convention. The working buffer layout places a
placeholder `0x00` at `buf[0]` and starts the actual wire report at `buf[1]`:

```text
hidapi buffer: 00 03 <opcode> ...
wire report:     03 <opcode> ...
```

If the captured leading `0x03` is placed at `buf[0]`, hidapi treats it as report
ID 3 and strips it, shifting the actual command left. Such writes can return
success while producing no visible effect.

## 3. Command summary

| Opcode | Name              | Status                            | Purpose                                             |
| -----: | ----------------- | --------------------------------- | --------------------------------------------------- |
| `0x20` | `REPORT`          | partly understood                 | Firmware/configuration/query family                 |
| `0x21` | `USER_ANIMATION`  | understood                        | Live or saved ring/head animation envelope          |
| `0x22` | `POWER_ANIMATION` | understood                        | Power-button persistent slot envelope               |
| `0x23` | `SELECT_ZONES`    | understood                        | Select one or more zone IDs for following actions   |
| `0x24` | `ADD_ACTION`      | understood                        | Append up to three 8-byte effect records            |
| `0x26` | `DIM`             | partly understood                 | Dim selected LED IDs / startup initialization       |
| `0x27` | `SET_COLOR`       | partly understood                 | Direct static-colour helper                         |
| `0x28` | `RESET`           | known from public implementations | Not used by AWCC reset-to-default path on this unit |
| `0xff` | `ERASE_FLASH`     | destructive                       | Do not use for RGB control                          |

## 4. Zone selection — `03:23`

Zone selection chooses the LED series that subsequent `03:24` action records
apply to.

```text
00 03 23 01 00 <count> <zone0> [<zone1> ...]
```

| Wire offset | Meaning                        |
| ----------: | ------------------------------ |
|      `0x00` | Fixed `0x03`                   |
|      `0x01` | Opcode `0x23`                  |
|      `0x02` | Constant `0x01` in known paths |
|      `0x03` | Reserved/unknown, write `0x00` |
|      `0x04` | Number of zone bytes following |
|    `0x05..` | Zone IDs                       |

Examples:

```text
00 03 23 01 00 01 00      ; select trackpad ring
00 03 23 01 00 01 02      ; select rear head
00 03 23 01 00 02 00 02   ; select ring and rear head together
00 03 23 01 00 01 04      ; select power button
```

Known zones:

|   Zone | Area                       | Required envelope |
| -----: | -------------------------- | ----------------- |
| `0x00` | Trackpad ring              | `03:21`           |
| `0x02` | Rear alien head / lid logo | `03:21`           |
| `0x04` | Power-button alien head    | `03:22`           |

A multi-zone selection applies each following `03:24` action sequence to all
selected live zones.

## 5. Effect action records — `03:24`

`03:24` appends one to three effect action records to the currently selected
zone set.

```text
00 03 24 <record0> [<record1>] [<record2>]
```

Each record is eight bytes:

```text
<mode> <duration_hi> <duration_lo> <tempo_hi> <tempo_lo> <R> <G> <B>
```

| Field      | Size | Meaning                                                                     |
| ---------- | ---: | --------------------------------------------------------------------------- |
| `mode`     |    1 | Primitive effect type                                                       |
| `duration` |    2 | Big-endian milliseconds                                                     |
| `tempo`    |    2 | Big-endian tempo/interpolation selector; high byte is `0x00` in known paths |
| `R G B`    |    3 | Colour tuple                                                                |

A `03:24` report holds at most three records, so longer effects span multiple
reports. A seven-keyframe rainbow uses three reports: 3 + 3 + 1 records.

### 5.1 Primitive modes

|   Mode | Name                    | Layout                                           |
| -----: | ----------------------- | ------------------------------------------------ |
| `0x00` | Static / `MODE_COLOR`   | `00 07 d0 00 fa <R G B>` for user static colours |
| `0x01` | Pulse / `MODE_PULSE`    | `01 07 d0 <tempo> <R G B>`                       |
| `0x02` | Keyframe / `MODE_MORPH` | `02 <duration:2> <tempo:2> <R G B>`              |

Static and pulse are single-record effects. Morph, Breathing, Spectrum, and
Rainbow Wave are lists of keyframes.

### 5.2 Duration

The duration field is big-endian milliseconds.

| Bytes   | Duration |
| ------- | -------: |
| `01 ac` |   428 ms |
| `01 f3` |   499 ms |
| `02 82` |   642 ms |
| `03 e8` |  1000 ms |
| `05 dc` |  1500 ms |
| `07 d0` |  2000 ms |
| `0b b7` |  2999 ms |
| `13 87` |  4999 ms |

Static and pulse records use fixed duration `07 d0` (2000 ms) in AWCC's known
paths.

### 5.3 Tempo

Known tempo values:

| Bytes   | Meaning in known paths                                                                                 |
| ------- | ------------------------------------------------------------------------------------------------------ |
| `00 64` | Fast/default pulse; preferred Morph/Breathing interpolation near black; system/default static contexts |
| `00 fa` | Slow pulse; user-selected static colours                                                               |
| `00 0f` | Rainbow/Spectrum keyframes                                                                             |

For pulse, lower numeric tempo is faster: `00 64` is fast/default and `00 fa` is
slow. For Morph/Breathing near black, `00 64` produces smoother visible output
than `00 fa` on the tested hardware.

### 5.4 Colour and brightness

Colour order is `R G B`. Brightness is encoded by scaling RGB values before
transmission; there is no separate brightness byte in `03:24` records.

Off is represented as static black:

```text
00 07 d0 00 fa 00 00 00
```

AW-ELC's default-colour sentinel is `00 f0 f0`. This is distinct from the
keyboard controller's default sentinel (`00 ff ff`).

## 6. Live animation envelope — `03:21`

Trackpad ring and rear head use `03:21`.

Wire layout:

```text
03 21 <sub_hi> <sub_lo> <animation_hi> <animation_lo> <duration_hi> <duration_lo>
```

Known subcommands:

|    Value | Name            | Meaning                                                           |
| -------: | --------------- | ----------------------------------------------------------------- |
| `0x0001` | `START_NEW`     | Start constructing an animation                                   |
| `0x0002` | `FINISH_N_SAVE` | Finish and save animation                                         |
| `0x0003` | `FINISH_N_PLAY` | Finish and play live animation                                    |
| `0x0004` | `REMOVE`        | Remove existing animation                                         |
| `0x0005` | `PLAY`          | Play existing animation / stop when ID is zero                    |
| `0x0006` | `SET_DEFAULT`   | Make animation the default live animation                         |
| `0x0007` | `SET_STARTUP`   | Make animation startup animation                                  |

For live, non-saved playback, AWCC/OpenRGB use animation ID `0xffff` and duration
`0x0000`.

### 6.1 Live transaction

A live transaction for ring/head is:

```text
00 03 21 00 01 ff ff 00 00   ; START_NEW, live slot
00 03 23 01 00 <count> ...   ; SELECT_ZONES
00 03 24 <records...>        ; ADD_ACTION, one or more reports
00 03 21 00 03 ff ff 00 00   ; FINISH_N_PLAY
```

Complete the transaction as one synchronous operation. Leaving a live animation
half-open while waiting for user input can strand the controller mid-update.

### 6.2 Saved animation transaction

The tested firmware stores the user animation in animation slot `0x0061`. The
observed save transaction is:

```text
00 03 21 00 04 00 61 00 00   ; REMOVE, delete existing saved animation
00 03 21 00 01 00 61 <dur>   ; START_NEW
00 03 23 ...                 ; SELECT_ZONES
00 03 24 ...                 ; ADD_ACTION records
00 03 21 00 02 00 61 <dur>   ; FINISH_N_SAVE
00 03 21 00 06 00 61 00 00   ; SET_DEFAULT
```

AWCC normally supplies an effect-specific duration. A duration of `0x0000` also
worked in hardware testing. The test used a roughly 350 ms settling delay around
`REMOVE` and `FINISH_N_SAVE`.

One slot can hold different effects for the ring and rear head. Select and write
each zone separately between the shared `START_NEW` and `FINISH_N_SAVE` calls.
The tested slot survived a full power-off, restored at boot without host USB
traffic, and replayed with `PLAY`.

## 7. Power-button state table — `03:22`

The power-button alien head is a firmware-managed power-state LED. The host
writes colours/effects into fixed persistent slots; firmware chooses which slot
to render according to AC/battery/sleep/charging/low-battery conditions.

Slot envelope:

```text
00 03 22 00 04 00 <slot>    ; REMOVE / open removal for this slot
00 03 22 00 01 00 <slot>    ; START_NEW
00 03 23 01 00 01 04        ; SELECT power-button zone
00 03 24 <effect records>   ; ADD_ACTION
00 03 22 00 02 00 <slot>    ; FINISH_N_SAVE
```

Known slots:

| Power condition  |   Slot | Action sequence          |
| ---------------- | -----: | ------------------------ |
| AC sleep         | `0x5b` | AC → black morph         |
| AC active        | `0x5c` | Static AC colour         |
| Charging         | `0x5d` | AC → battery morph       |
| Battery sleep    | `0x5e` | Battery → black morph    |
| Battery active   | `0x5f` | Static battery colour    |
| Battery critical | `0x60` | Pulsing battery colour   |

AC and battery are the two configured base colours. Charging is a derived
firmware condition, not a third base colour: slot `0x5d` needs both endpoints
in one two-keyframe morph. Changing either base colour therefore also requires
rebuilding `0x5d`. When both colours change together, write the charging slot
once after resolving both new values.

Luminate exposes the physical alien head as one `power-button` surface with
appearance slots `ac` and `battery`. These API slots are base-colour programs,
not a one-to-one exposure of the six firmware records above. The grouped API
operation lets the plugin derive the sleep, charging, active, and critical
records without pretending that firmware's `0x5d` charging program is
independently writable.

Do not rebuild `0x5d` from a guessed endpoint when one base colour is unknown.
Preserve the existing slot and request both colours together instead. Partial
or single-keyframe updates can produce confusing firmware-managed behaviour.

Actual AC/battery transitions do not require host USB traffic. Once slots are
stored, firmware switches rendered behaviour autonomously.

## 8. Report/query command — `03:20`

`03:20` is a query family. Known subcommands:

| Subcommand | Meaning                                | Known response fields                           |
| ---------: | -------------------------------------- | ----------------------------------------------- |
|     `0x00` | Firmware version                       | Response bytes `4..6` are `major.minor.patch`   |
|     `0x02` | Firmware configuration                 | Bytes `4..5` platform ID; byte `6` total LEDs   |
|     `0x03` | Animation count                        | Bytes `4..5` count; byte `6` maximum ID         |
|     `0x04` | Animation metadata / animation ID read | Request construction appears unreliable in AWCC |
|     `0x05` | Series LED-index read                  | Request construction appears unreliable in AWCC |

The examined AWCC API does not expose a reliable read-back for currently
rendered RGB state or active live animation contents, so software that needs to
preserve them has to maintain shadow state.

On the tested m16 R2, subcommand `0x02` returns platform ID `0x1102` and a raw
zone count of five. The count describes the firmware zone-ID range rather than
five known lighting surfaces: AWCC defines IDs `0x00` (touchpad surround),
`0x02` (rear Alienhead), and `0x04` (power-button/status LED), while IDs `0x01`
and `0x03` are undefined. Luminate does not expose or write the undefined IDs.

### 8.1 Read-only configuration identity

Luminate sends this complete hidapi buffer before publishing an AW-ELC device:

```text
00 03 20 02 00 ... 00
```

It then reads a feature report and requires at least seven bytes. Offsets here
include the hidapi placeholder:

| Buffer offset | Meaning |
| ------------: | ------- |
| `0` | Placeholder/report-ID byte, expected `0x00` |
| `1` | Wire report marker, expected `0x03` |
| `2` | Opcode, expected `0x20` |
| `3` | Configuration subcommand, expected `0x02` |
| `4..5` | Big-endian platform ID |
| `6` | Raw firmware zone count |

Wrong markers, short reports, unknown platform IDs, and unexpected counts are
identity failures. They do not fall back to another model or a generic zone
range. If more than one supported USB candidate is attached, discovery also
withholds all AW-ELC topology because the public device ID does not encode a
stable controller instance.

### 8.2 Corroborated platform table

The following `187c:0550` identities and logical zone tables come from OpenRGB
revision `5e6d627f519a791487e766aeee51eed7bf7129ef`. They are recognized by
Luminate but remain externally corroborated until each named laptop completes
hardware validation:

| Platform | Model | Expected raw count | Corroborated logical zones |
| -------: | ----- | -----------------: | -------------------------- |
| `0x0c01` | Dell G5 SE 5505 | 4 | Left, middle, right, numpad (`0x00..0x03`) |
| `0x0a01` | Dell G7 15 7500 | 16 | Four keyboard regions and twelve light-bar segments (`0x00..0x0f`) |
| `0x0e03` | Dell G15 5511 | 4 | Left, middle, right, numpad (`0x00..0x03`) |
| `0x0e0a` | Dell G15 5530 | 4 | Left, middle, right, numpad (`0x00..0x03`) |

This evidence establishes controller identity, logical ordering, and use of the
live-animation report family. It does not establish per-key layout, coordinates,
power-button slots, saved-animation identity, boot restoration, or exact
readback on these models. Luminate consequently exposes their zones as opaque,
volatile live surfaces and does not offer appearance slots or `SaveCurrent`.
The m16 R2 `187c:0551`, platform `0x1102`, remains the only profile in this
document whose topology, power-button semantics, and persistence have been
hardware-validated in Luminate.

## 9. Direct LED helpers

### 9.1 DIM — `03:26`

```text
03 26 <dim_level> <page> <count> <led_id0> [<led_id1> ...]
```

Up to 28 LED IDs fit in one packet. AWCC's implementation queries the total LED
count, excludes requested LEDs, splits the remainder into pages, and sends one
packet per page. On the tested m16 R2 this appeared in startup initialization,
not ordinary GUI brightness control.

### 9.2 SET_COLOR — `03:27`

```text
03 27 <R> <G> <B> 00 <count> <led_id0> [<led_id1> ...]
```

Up to 26 LED IDs fit in one packet. This is a direct static-colour helper
implemented by AWCC, but ordinary captured GUI paths use `03:23`/`03:24`
animation records instead.

## 10. Named effects

### 10.1 Static

One static record:

```text
00 07 d0 00 fa <R G B>
```

For AWCC's system default static colour (`00 f0 f0`), the canonical tempo is
`00 64` rather than `00 fa`:

```text
00 07 d0 00 64 00 f0 f0
```

### 10.2 Pulse

One pulse record:

```text
01 07 d0 <tempo_hi> <tempo_lo> <R G B>
```

Known pulse speeds:

| Speed        | Tempo   |
| ------------ | ------- |
| Fast/default | `00 64` |
| Slow         | `00 fa` |

### 10.3 Morph

Two or more keyframes:

```text
02 <duration> <tempo> <R0 G0 B0>
02 <duration> <tempo> <R1 G1 B1>
...
```

### 10.4 Breathing

Breathing is a two-keyframe morph from colour to black:

```text
02 05 dc 00 64 <R G B>
02 05 dc 00 64 00 00 00
```

Morphing to a small non-zero endpoint can reduce low-brightness stepping if true
black is not required.

### 10.5 Spectrum

Spectrum uses the fixed seven-colour palette in the same phase on all selected
zones.

| Stop | RGB        |
| ---: | ---------- |
|    0 | `ff 00 00` |
|    1 | `ff a5 00` |
|    2 | `ff ff 00` |
|    3 | `00 80 00` |
|    4 | `00 bf ff` |
|    5 | `00 00 ff` |
|    6 | `80 00 80` |

Observed medium Spectrum timing:

```text
duration = 02 82
tempo    = 00 0f
```

### 10.6 Rainbow Wave

Rainbow Wave uses the same seven-colour palette as Spectrum, but phase-rotates
per zone. For the observed ring/head pair:

```text
head[i] = ring[(i + 5) mod 7]
```

Observed Rainbow Wave timing:

```text
duration = 01 ac
tempo    = 00 0f
```

The general phase rule for controllers with more than two live zones is not
known.

### 10.7 Reset to Default

AWCC's "Reset to Default" for ring/head is not opcode `0x28`. It is an ordinary
multi-zone static write to zones `0x00` and `0x02` using default sentinel colour
`00 f0 f0`:

```text
00 03 21 00 01 ff ff 00 00
00 03 23 01 00 02 00 02
00 03 24 00 07 d0 00 64 00 f0 f0
00 03 21 00 03 ff ff 00 00
```

## 11. Save Preset and profile switching

AWCC Save Preset persists both command families:

1. Rewrites the six power-button slots `0x005b..0x0060` through `03:22`.
2. Saves the ring/head animation as user animation `0x0061` through `03:21`.
3. Sets animation `0x0061` as default with subcommand `0x0006`.

Game-profile switching performed the full save transaction twice, byte-for-byte
identical and roughly 8–9 seconds apart. This duplication comes from AWCC, not
the controller protocol.

The save and live transactions are independent. Temporary changes do not need
the persistence sequence.

The live `0xffff` transaction suits quick temporary changes. Firmware
persistence instead uses the saved-animation and power-slot paths.

## 12. Practical implementation notes

Known-good software for this controller does the following:

1. Use the hidapi placeholder byte (`00`) before all 33-byte wire reports.
2. Pace writes; the controller's USB stack can fail if large transactions are
   sent too quickly.
3. Complete live `03:21` transactions promptly: open, select, add all actions,
   commit.
4. Keep shadow state for ring, rear head, and power button. No reliable rendered
   RGB read-back is known.
5. Re-push both live zones when emulating AWCC or when preserving the unchanged
   zone matters.
6. Program the power button only through `03:22` fixed slots.
7. Update all three slots for a power-state group when changing power-button
   colour.
8. Avoid opcode `0xff`; it is destructive flash erase.

## 13. Examples

All examples include the hidapi placeholder byte. Pad them with zeroes to 34
bytes total before sending them through hidapi.

### 13.1 Ring static red

```text
00 03 21 00 01 ff ff 00 00
00 03 23 01 00 01 00
00 03 24 00 07 d0 00 fa ff 00 00
00 03 21 00 03 ff ff 00 00
```

### 13.2 Ring pulse red, fast/default

```text
00 03 21 00 01 ff ff 00 00
00 03 23 01 00 01 00
00 03 24 01 07 d0 00 64 ff 00 00
00 03 21 00 03 ff ff 00 00
```

### 13.3 Ring Morph red to blue, 1.5-second stops

```text
00 03 21 00 01 ff ff 00 00
00 03 23 01 00 01 00
00 03 24 02 05 dc 00 64 ff 00 00 \
         02 05 dc 00 64 00 00 ff
00 03 21 00 03 ff ff 00 00
```

### 13.4 Ring Breathing red

```text
00 03 21 00 01 ff ff 00 00
00 03 23 01 00 01 00
00 03 24 02 05 dc 00 64 ff 00 00 \
         02 05 dc 00 64 00 00 00
00 03 21 00 03 ff ff 00 00
```

### 13.5 Ring and rear head default cyan/blue

```text
00 03 21 00 01 ff ff 00 00
00 03 23 01 00 02 00 02
00 03 24 00 07 d0 00 64 00 f0 f0
00 03 21 00 03 ff ff 00 00
```

### 13.6 Power button AC green, battery amber

Program each condition separately. AC sleep in slot `0x5b` fades green to
black:

```text
00 03 22 00 04 00 5b
00 03 22 00 01 00 5b
00 03 23 01 00 01 04
00 03 24 02 03 e8 00 64 00 ff 00 \
         02 03 e8 00 64 00 00 00
00 03 22 00 02 00 5b
```

Slot `0x5c` uses static green. Charging slot `0x5d` morphs from AC green to
battery amber:

```text
00 03 22 00 04 00 5d
00 03 22 00 01 00 5d
00 03 23 01 00 01 04
00 03 24 02 03 e8 00 64 00 ff 00 \
         02 03 e8 00 64 ff bf 00
00 03 22 00 02 00 5d
```

## 14. Confirmed working paths

Hardware testing covered:

- Ring: static, pulse, two-colour Morph, Breathing, Spectrum, Rainbow Wave.
- Rear head: static, Spectrum, Rainbow Wave.
- Multi-zone ring+rear selection for static, static black, default sentinel,
  Spectrum, and Rainbow Wave.
- Pulse tempo mapping (`00 64` fast, `00 fa` slow).
- Breathing/morph using `00 64` for smoother low-end fades.
- Power-button static colour via the complete persistent slot set.
- Saved-animation persistence for different ring and rear-head effects in slot
  `0x0061`, including automatic restoration after a full reboot and replay with
  `PLAY` (tested 2026-07-15).

## 15. Open protocol items

- Byte-level semantics of responses to `03:20` subcommands `0x04` and `0x05`.
- Whether `03:26 DIM` is useful outside startup initialization on this model.
- Which production path, if any, uses direct `03:27 SET_COLOR`.
- General Rainbow Wave phase assignment for more than two live zones.
- Hardware validation of the sleep fades and two-endpoint charging morph after
  programming them outside AWCC.
- Whether an undocumented opcode can report currently rendered RGB state.
