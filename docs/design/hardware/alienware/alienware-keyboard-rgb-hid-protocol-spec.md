---
# SPDX-License-Identifier: CC-BY-SA-4.0
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford
title: Alienware m16 R2 Keyboard RGB Hardware Notes
author:
  - Elizabeth Ashford
date: 2026-07-09
version: 1.0
description: Reverse-engineered hardware notes for the Alienware m16 R2 keyboard RGB controller.
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

# Alienware m16 R2 Keyboard RGB Hardware Notes

These notes describe the keyboard RGB protocol used by the Alienware m16 R2
US-ANSI internal keyboard controller observed as VID:PID `0x0D62:0xD2B1`
(`Darfon Electronics Corp. Keyboard`). It is based on USB captures, direct
hardware testing, and static analysis of the Darfon implementation shipped with
Alienware Command Center (`DARFON.Core.dll` / `Intrepid.FX.API.Core` 1.65.5,
with transport delegated to `ITECCTdll.dll`).

The command family matches the public AlienFX APIv5 shape: fixed leading byte
`0xCC`, 64-byte HID reports, and 8-bit RGB channel values. The physical key map
and some AWCC-specific behaviours are specific to the tested m16 R2 US-ANSI
keyboard. Other models and regional layouts may use different masks or aliases.

## Reading the packet descriptions

- Bytes are written as lowercase hexadecimal unless part of copied examples.
- Multi-byte integer endianness is stated per command. Where it is unknown, the
  notes leave it unknown rather than borrowing the little-endian convention from
  other Alienware protocols.
- `cc:<opcode>` denotes a 64-byte report whose first byte is `0xCC` and whose
  second byte is `<opcode>`.
- `R G B` denotes byte order red, green, blue.
- Unspecified bytes in examples are zero-filled to the report length.
- Known-good writes use zero for fields marked **reserved**. Preserving bytes
  read from an AWCC sequence is another reasonable choice where noted.

This is an informal account of reverse-engineered hardware, not a compatibility
standard or a claim about Dell's internal API stability. Terms such as "known",
"partly understood", and "unresolved" describe the evidence available from the
tested machine.

## 1. Device model

The keyboard exposes two independent lighting engines.

| Engine                                  | Command family | Purpose                                                         |
| --------------------------------------- | -------------- | --------------------------------------------------------------- |
| Custom animation / per-key colour table | `cc:8c:*`      | Per-key static colours and firmware custom-effect slots         |
| Built-in whole-keyboard effects         | `cc:80`        | Static solid colour, Breathing, Rainbow Wave, Scanner, Spectrum |

The engines retain independent state. Switching to a built-in `cc:80` effect can
leave the previous per-key table intact, but that table is not rendered while
the built-in effect engine is active. For reliable per-key control, initialize
the custom-animation path explicitly before sending `cc:8c:02` colour data.

## 2. Transport

Host-to-device commands use HID `SET_REPORT` control transfers. The report body
is 64 bytes and begins with `0xCC`.

| Field                 | Value                              |
| --------------------- | ---------------------------------- |
| Controller            | `0x0D62:0xD2B1` Darfon keyboard    |
| Command report length | 64 bytes                           |
| Fixed first byte      | `0xCC`                             |
| Main write channel    | HID `SET_REPORT` control transfer  |
| Read-back channel     | HID `GET_REPORT` for report `0xCC` |

AWCC also sends a two-byte report-ID-1 keyboard LED output (`01 00`) near some
transactions. It belongs to normal keyboard LED state, not AlienFX, and is not
needed for RGB control.

## 3. Command summary

| Command                   | Status                    | Purpose                                                                                                                   |
| ------------------------- | ------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `cc:80`                   | understood                | Run built-in whole-keyboard effect                                                                                        |
| `cc:83`                   | partly understood         | All-key brightness / effect finalizer                                                                                     |
| `cc:84`                   | partly understood         | Persist active keyboard effect                                                                                            |
| `cc:85`                   | unresolved                | Fixed boot/startup command emitted by AWCC                                                                                |
| `cc:8b`                   | partly understood         | Commit/apply custom-animation updates                                                                                     |
| `cc:8c:01`                | understood for known slots | Configure custom firmware effect slot                                                                                    |
| `cc:8c:02`                | understood                | Upload per-key RGB records                                                                                                |
| `cc:8c:05..07`            | understood                | Upload 140-entry slot assignment table                                                                                    |
| `cc:8c:10`                | partly understood         | Stops the currently running custom engine and clears firmware effect slots prior to uploading a new custom configuration. |
| `cc:8c:13`                | partly understood         | End/finalize custom-animation table upload                                                                                |
| `cc:93`                   | partly understood         | Hardware/layout identity read-back                                                                                        |
| `cc:94`                   | partly understood         | Current operating-state read-back                                                                                         |
| `cc:96`, `cc:73`, `cc:79` | AWCC-private              | Preset/profile storage orchestration                                                                                      |
| `cc:76`, `cc:99`, `cc:9c` | unresolved                | Startup-only commands observed from AWCC                                                                                  |

Normal RGB control only needs the commands marked understood or partly
understood. The AWCC-private and unresolved commands are relevant mainly when
studying or reproducing AWCC's profile-store behaviour.

## 4. Built-in whole-keyboard effect command — `cc:80`

`cc:80` starts or updates one whole-keyboard built-in effect.

```text
cc 80 <effect> <param_a> 00 00 <flag0> <flag1> <flag2> <flag3> \
      <r1> <g1> <b1> <r2> <g2> <b2> <param_b> 00 ...
```

|       Offset | Size | Meaning                                  |
| -----------: | ---: | ---------------------------------------- |
|       `0x00` |    1 | Fixed `0xCC`                             |
|       `0x01` |    1 | Opcode `0x80`                            |
|       `0x02` |    1 | Effect ID                                |
|       `0x03` |    1 | Effect-specific speed/timing parameter A |
| `0x04..0x05` |    2 | Reserved, write zero                     |
| `0x06..0x09` |    4 | Effect flags                             |
| `0x0A..0x0C` |    3 | Primary RGB                              |
| `0x0D..0x0F` |    3 | Secondary RGB                            |
|       `0x10` |    1 | Effect-specific speed/timing parameter B |
| `0x11..0x3F` |   47 | Reserved, write zero                     |

### 4.1 Effect IDs

| Effect                 |     ID | Notes                                                  |
| ---------------------- | -----: | ------------------------------------------------------ |
| Static solid colour    | `0x01` | AWCC labels the default instance "Static Default Blue" |
| Breathing              | `0x02` | Fixed-colour effect                                    |
| Rainbow Wave           | `0x03` | Auto-cycling effect; colour fields ignored/placeholder |
| Scanner / Knight Rider | `0x0A` | Fixed-colour effect                                    |
| Spectrum               | `0x0E` | Auto-cycling effect; colour fields ignored/placeholder |

Effect `0x01` is a general solid-colour effect. The default AWCC colour happens
to be Alienware cyan/blue; the effect honours any supplied RGB tuple.

### 4.2 Colour fields

Colour byte order is `R G B`. The GUI brightness/lightness value is folded into
the RGB tuple before transmission; there is no separate brightness field in
`cc:80`.

For fixed-colour effects, the primary colour is the user-selected colour. The
secondary colour is also present. Known-good packets keep it consistent with
the primary colour unless a specific effect mapping says otherwise.

For Rainbow Wave and Spectrum, AWCC sends placeholder red (`ff 00 00`) and the
firmware generates the displayed colours internally.

When a fixed-colour effect has never had a user colour selected in the current
AWCC state, AWCC uses `00 ff ff` as the keyboard's default-colour sentinel. When
a previously selected colour is explicitly cleared, AWCC writes `00 00 00`.
Treat `00 ff ff` as a device-default placeholder, not proof that the user
selected cyan.

### 4.3 Flags

| Effect class                                  | `0x06..0x09`  |
| --------------------------------------------- | ------------- |
| Fixed-colour effects (`0x01`, `0x02`, `0x0A`) | `01 01 01 00` |
| Auto-cycling effects (`0x03`, `0x0E`)         | `01 01 01 01` |

The first three flag bytes are always `01` in the captured full-keyboard
configuration. The fourth flag selects fixed-colour versus auto-cycling mode for
known effects.

### 4.4 Speed fields

Offsets `0x03` and `0x10` are effect-specific timing selectors. They are lookup
values, not milliseconds.

Known defaults:

| Effect                                 |          Offset `0x03` |                           Offset `0x10` |
| -------------------------------------- | ---------------------: | --------------------------------------: |
| Breathing, default                     |                 `0x07` |                                  `0x05` |
| Scanner, default                       |                 `0x07` |                                  `0x06` |
| Rainbow Wave, low/medium/high examples | `0x02`, `0x05`, `0x09` | low/medium match, high clamps at `0x06` |
| Spectrum, medium/fast examples         |         `0x01`, `0x02` |                         effect-specific |

The two values do not follow one general formula; per-effect timing tables match
the observed behaviour.

## 5. Custom-animation path

The custom-animation path uses opcode `cc:8c` with subcommands. It handles both
per-key Custom Colour and the firmware custom-effect slots.

A full custom-colour apply generally consists of:

```text
cc 94 ...                  ; read/open state block, optional for minimal client
cc 8c 10 00 ...            ; reset/clear custom animation, when changing engines
cc 8c 01 <slot> ...        ; clear/configure slots
cc 8c 05 ...               ; assignment table entries 0..59
cc 8c 06 ...               ; assignment table entries 60..119
cc 8c 07 ...               ; assignment table entries 120..139
cc 8c 02 ...               ; per-key RGB records, one or more reports
cc 8c 13 00 ...            ; finalize table upload
cc 8b 01 ff ...            ; commit/apply
```

AWCC may interleave `GET_REPORT` reads after `cc:94`, `cc:93`, or commits.
Those reads are useful for diagnostics but are not required by a minimal writer.

### 5.1 Slot configuration — `cc:8c:01`

```text
cc 8c 01 <slot> <timing_a> 00 00 <direction> <trigger> <enabled> \
      <effect_flag> <r1> <g1> <b1> <r2> <g2> <b2> <timing_b> 00 ...
```

|       Offset | Meaning                                        |
| -----------: | ---------------------------------------------- |
|       `0x03` | Firmware effect slot                           |
|       `0x04` | Effect-specific timing selector A              |
| `0x05..0x06` | Reserved / unknown, write zero for known paths |
|       `0x07` | Direction                                      |
|       `0x08` | Trigger                                        |
|       `0x09` | Enabled / loop-related flag                    |
|       `0x0A` | Effect-specific flag                           |
| `0x0B..0x0D` | Primary RGB                                    |
| `0x0E..0x10` | Secondary RGB                                  |
|       `0x11` | Effect-specific timing selector B              |

Known slots:

|   Slot | Meaning                       |
| -----: | ----------------------------- |
| `0x00` | Unassigned                    |
| `0x01` | Static/custom colour          |
| `0x02` | Breathing                     |
| `0x05` | Reactive/key-triggered effect |
| `0x08` | Reserved initialization slot  |
| `0x09` | Reserved initialization slot  |
| `0x0E` | Spectrum                      |

Slot `0x05` is also used internally by AWCC's reactive lighting implementation.

Known AWCC predefined IDs:

| AWCC predefined ID | AWCC name              | Firmware slot |
| -----------------: | ---------------------- | ------------: |
|                `7` | Breathing              |        `0x02` |
|                `8` | Spectrum               |        `0x0E` |
|                `9` | Internal reactive path |        `0x05` |

Direction values:

|  Value | Direction                         |
| -----: | --------------------------------- |
| `0x01` | None / right-to-left              |
| `0x02` | Left-to-right                     |
| `0x03` | Down-to-up                        |
| `0x04` | Up-to-down                        |
| `0x05` | Right-to-left, then left-to-right |
| `0x06` | Left-to-right, then right-to-left |

Trigger values:

|  Value | Trigger          |
| -----: | ---------------- |
| `0x01` | Immediate/static |
| `0x02` | Key press        |
| `0x03` | Key release      |

A uniform static custom-colour slot uses:

```text
cc 8c 01 01 01 00 00 01 01 01 00 <R G B> <R G B> 01
```

### 5.2 Slot assignment table — `cc:8c:05`, `cc:8c:06`, `cc:8c:07`

The assignment table maps AWCC's 140-position canonical matrix to firmware
effect slots. Each byte is a slot number; zero means unassigned.

```text
cc 8c <05|06|07> 00 <assignment bytes...>
```

| Report     | Entries carried                                           |
| ---------- | --------------------------------------------------------- |
| `cc:8c:05` | Matrix positions `0..59`                                  |
| `cc:8c:06` | Matrix positions `60..119`                                |
| `cc:8c:07` | Matrix positions `120..139` in the first 20 payload bytes |

A static Custom Colour setup normally assigns populated positions to slot
`0x01`, then supplies per-key RGB records through `cc:8c:02`.

### 5.3 Per-key RGB table — `cc:8c:02`

```text
cc 8c 02 00 <idx> <R> <G> <B> ...
```

|       Offset | Meaning                                            |
| -----------: | -------------------------------------------------- |
|       `0x00` | Fixed `0xCC`                                       |
|       `0x01` | Opcode `0x8C`                                      |
|       `0x02` | Subcommand `0x02`                                  |
|       `0x03` | Reserved, write zero                               |
| `0x04..0x3F` | Up to 15 4-byte records: `<key_index> <R> <G> <B>` |

A record of `00 00 00 00` is padding, not key index zero. The tested keyboard
uses 85 physical keys, so a full-board upload spans multiple reports.

Colour order is `R G B`. The colour bytes are the values the host chooses to
upload after applying any desired brightness scaling, white-balance, or profile
logic. The firmware does not expose a separate per-key brightness field on this
path.

### 5.4 Brightness and white handling

For Custom Colour, host software encodes brightness by scaling each key's RGB
bytes before writing `cc:8c:02`.

AWCC applies key-dependent calibration. Some keys do not receive the literal
requested full-scale value even at GUI 100%. The Escape/F-row zone, for example,
uses approximately 35–45% of the full byte range in captures.

AWCC also white-balances GUI `#FFFFFF` rather than sending `ff ff ff`. The
full-brightness compensated ratio is approximately:

```text
R : G : B = 0.667 : 1.000 : 0.824
example full-scale tuple: aa ff d2
```

Reserved or unpopulated matrix positions can echo literal `ff ff ff` because no
physical LED calibration is applied to them. Known unpopulated white-echo
indices on the tested US-ANSI unit are:

```text
0x22, 0x37, 0x3c, 0x4a, 0x53, 0x6a, 0x6f
```

### 5.5 All-key brightness / finalizer — `cc:83`

```text
cc 83 38 9c <level> 00 ...
```

Static analysis names this command `ALLKeyPerLEDBriLevelcontrol()`. `<level>`
is scaled as:

```text
level = round(percent * 254 / 100)
```

Nominal 100% is `0xfe`. The constants `0x38` and `0x9c` are not identified.
AWCC sends this command after built-in `cc:80` transactions but not
after ordinary per-key Custom Colour writes. In practice it serves as both an
all-key brightness write and an effect-engine finalizer for `cc:80` paths.

## 6. State read-back

### 6.1 Current operating state — `cc:94`

AWCC reads a state block selected by `cc:94`. Known response fields:

| Response offset | Meaning                                                              |
| --------------: | -------------------------------------------------------------------- |
|          `0x02` | Current lighting mode; `0x8C` denotes custom-animation mode          |
|          `0x13` | Current macro bank                                                   |
|          `0x15` | Current global brightness byte                                       |
|    `0x16..0x18` | Base/current colour-related bytes used by AWCC when preserving state |

### 6.2 Hardware/layout identity — `cc:93`

Known response fields:

| Response offset | Meaning                                   |
| --------------: | ----------------------------------------- |
|    `0x02..0x03` | Hardware/model/platform variant selectors |
|          `0x04` | Keyboard layout enum                      |
|          `0x05` | Device/chassis colour variant             |

The layout enum returned by `cc:93` is used internally by the Darfon
implementation to select layout-specific key masks and calibration tables.

`cc:93` and `cc:94` do not provide a complete current per-key RGB table. Keep a
shadow copy of any per-key state that needs to survive later writes.

## 7. Persistence

The low-level keyboard persistence command is:

```text
cc 84 03 00 00 ...
```

AWCC calls this from `SaveCurrentEffectsToLastEffect()` after applying an
effect. Sending `cc:84:03:00` after a successful apply persists that effect.

AWCC's Save Preset path also emits `cc:96`, `cc:73`, and `cc:79` traffic. These
appear to orchestrate AWCC profile storage rather than directly programming the
keyboard controller.

Hardware testing confirmed persistence across a complete power-off and reboot.

AWCC also exposes a fixed boot/startup command:

```text
cc 85 ae 00 ...
```

The Darfon method name is `SetBootupAnimation(int)`, but the argument is ignored
and the transmitted value is always `0xae`. Its precise firmware semantics are
not known.

## 8. Physical key index map

These are the canonical lower-bank addresses on the tested US-ANSI keyboard.

### 8.1 First row

| Key    |  Index |
| ------ | -----: |
| Escape | `0x01` |
| F1     | `0x02` |
| F2     | `0x03` |
| F3     | `0x04` |
| F4     | `0x05` |
| F5     | `0x06` |
| F6     | `0x07` |
| F7     | `0x08` |
| F8     | `0x09` |
| F9     | `0x0a` |
| F10    | `0x0b` |
| F11    | `0x0c` |
| F12    | `0x0d` |
| Home   | `0x0e` |
| End    | `0x0f` |
| Delete | `0x10` |

### 8.2 Second row

| Key       |  Index |
| --------- | -----: |
| Mic Mute  | `0x14` |
| `` ` ``   | `0x15` |
| 1         | `0x16` |
| 2         | `0x17` |
| 3         | `0x18` |
| 4         | `0x19` |
| 5         | `0x1a` |
| 6         | `0x1b` |
| 7         | `0x1c` |
| 8         | `0x1d` |
| 9         | `0x1e` |
| 0         | `0x1f` |
| `-`       | `0x20` |
| `=`       | `0x21` |
| Backspace | `0x24` |

### 8.3 Third row

| Key         |  Index |
| ----------- | -----: |
| Volume Mute | `0x11` |
| Tab         | `0x29` |
| Q           | `0x2b` |
| W           | `0x2c` |
| E           | `0x2d` |
| R           | `0x2e` |
| T           | `0x2f` |
| Y           | `0x30` |
| U           | `0x31` |
| I           | `0x32` |
| O           | `0x33` |
| P           | `0x34` |
| `[`         | `0x35` |
| `]`         | `0x36` |
| `\`         | `0x38` |

### 8.4 Fourth row

| Key         |  Index |
| ----------- | -----: |
| Volume Down | `0x12` |
| Volume Up   | `0x13` |
| Caps Lock   | `0x3e` |
| A           | `0x3f` |
| S           | `0x40` |
| D           | `0x41` |
| F           | `0x42` |
| G           | `0x43` |
| H           | `0x44` |
| J           | `0x45` |
| K           | `0x46` |
| L           | `0x47` |
| `;`         | `0x48` |
| `'`         | `0x49` |
| Enter       | `0x4b` |

### 8.5 Fifth row

| Key         |  Index |
| ----------- | -----: |
| Left Shift  | `0x52` |
| Z           | `0x54` |
| X           | `0x55` |
| C           | `0x56` |
| V           | `0x57` |
| B           | `0x58` |
| N           | `0x59` |
| M           | `0x5a` |
| `,`         | `0x5b` |
| `.`         | `0x5c` |
| `/`         | `0x5d` |
| Right Shift | `0x5f` |
| Up          | `0x73` |

### 8.6 Sixth row

| Key        |  Index |
| ---------- | -----: |
| Left Ctrl  | `0x65` |
| Fn         | `0x66` |
| Left Win   | `0x68` |
| Left Alt   | `0x69` |
| Space      | `0x6c` |
| Right Win  | `0x6e` |
| Right Alt  | `0x70` |
| Right Ctrl | `0x71` |
| Left       | `0x86` |
| Down       | `0x87` |
| Right      | `0x88` |

## 9. Aliases and sparse/gap positions

### 9.1 Lower-bank aliases

Some physical keys respond to more than one lower-bank index.

| Physical key | Indices        |
| ------------ | -------------- |
| Backspace    | `0x23`, `0x24` |
| Left Shift   | `0x51`, `0x52` |
| Space        | `0x6b`, `0x6c` |
| Left Win     | `0x67`, `0x68` |

Aliases address the same physical LED output. If conflicting colours for the
same LED are sent within one `cc:8c:02` upload, the last record in report order
wins. There is no known priority rule based on numeric index.

Choosing one canonical index per physical key avoids conflicting alias records.

### 9.2 Upper alias bank

Indices `0x8d..0xff` form a mostly mirrored alias bank. Most working entries
map to a lower index by subtracting `0x8c`.

Examples:

|  Upper |  Lower | Key     |
| -----: | -----: | ------- |
| `0x8d` | `0x01` | Escape  |
| `0xa1` | `0x15` | `` ` `` |
| `0xb7` | `0x2b` | Q       |
| `0xd7` | `0x4b` | Enter   |
| `0xf7` | `0x6b` | Space   |
| `0xff` | `0x73` | Up      |

The mirror is table-driven rather than arithmetic wrapping. The following upper
indices are dead on the tested unit:

| Dead upper index | Lower candidate | Would-be key               |
| ---------------: | --------------: | -------------------------- |
|           `0xaf` |          `0x23` | Backspace secondary alias  |
|           `0xc4` |          `0x38` | Backslash                  |
|           `0xdd` |          `0x51` | Left Shift secondary alias |
|           `0xf3` |          `0x67` | Left Win secondary alias   |
|           `0xfc` |          `0x70` | Right Alt                  |

Left, Down, and Right cannot have upper mirrors because adding `0x8c` exceeds
`0xff`. Up (`0x73 -> 0xff`) is the final populated upper alias.

### 9.3 Reserved positions

The matrix contains unpopulated positions for other layouts or chassis variants.
Important reserved lower-bank positions include:

|  Index | Position                  | Likely purpose                              |
| -----: | ------------------------- | ------------------------------------------- |
| `0x22` | Between `=` and Backspace | JIS `¥`, split Backspace, or layout padding |
| `0x37` | Between `]` and `\`       | Regional-layout symbol key                  |
| `0x3c` | Gap before Caps Lock      | Wider/numpad SKU reserve                    |
| `0x4a` | Between `'` and Enter     | ISO Enter extra segment                     |
| `0x53` | Between Left Shift and Z  | ISO extra key next to Left Shift            |
| `0x6a` | Bottom-row gap            | Wider/numpad or modifier reserve            |
| `0x6f` | Near Right Win/Right Ctrl | Menu/Application key candidate              |

Static analysis shows that the firmware maps one 140-position matrix onto US,
UK, German, French, Spanish LATAM, Nordic, Japanese, Korean, Canadian bilingual,
and related layouts. Gaps on the US-ANSI keyboard can represent keys on other
layouts. Evidence is strongest for `0x4a` (the ISO/JIS Enter region) and `0x53`
(the ISO key beside Left Shift); the other proposed mappings remain unconfirmed.

## 10. AWCC GUI grouping metadata

These groups belong to the AWCC interface, not the firmware protocol.

| GUI group     | Member indices                       |
| ------------- | ------------------------------------ |
| Function Keys | `0x02..0x0d` plus `0x14`             |
| Numbers       | `0x16..0x1f`                         |
| QWER          | `0x2b,0x2c,0x2d,0x2e,0x3f,0x40,0x41` |
| WASD          | Same seven-key set as QWER           |

Selecting a named group in AWCC changes the ordering of the next full resend,
but deselecting a named group emits no USB traffic. Selecting individual keys in
AWCC's per-key grid follows a different application path and can cause multiple
rapid resends.

## 11. Profile switching and sparse writes

Normal Custom Colour editing resends the full per-key table. AWCC game-profile
switching can instead emit sparse `cc:8c:02` writes after profile/cache
orchestration. Captured sparse profile writes included gap/reserved positions,
which implies the path writes against the shared canonical matrix rather than
only populated physical keys.

For deterministic updates, upload a complete table and maintain a shadow copy
instead of relying on AWCC's sparse profile-switch sequence. A table may require
several `cc:8c:02` reports, so the visible update is immediate but not atomic.

## 12. Startup-only traffic

When AWCC's background agent starts, it emits additional commands not observed
in ordinary UI writes:

```text
cc 99 00 00 00 00 00 ...
cc 76 00 00 00 00 00 ...
cc 9c 80 00 00 00 00 00 ...
```

These commands are reproducible but unresolved. Basic RGB control works without
them, and sending them without first understanding their semantics adds risk for
no known benefit.

## 13. Practical implementation notes

Known-good software for this keyboard does the following:

1. Treat `cc:80` and `cc:8c` as separate engines.
2. Explicitly initialize or switch to the custom-animation engine before
   uploading per-key colour data.
3. Maintain a shadow copy of per-key state; no complete colour-table read-back
   is known.
4. Upload complete per-key tables for normal state changes.
5. Avoid conflicting alias records; last-written record wins within a report.
6. Zero-fill reserved bytes.
7. Use per-model/layout key maps and presence masks instead of deriving physical
   keys from contiguous indices.
8. Persist the active keyboard effect with `cc:84:03:00` only after the desired
   effect has been applied successfully.

## 14. Open protocol items

- Full meaning of `cc:8c:01` offsets `0x05`, `0x06`, `0x09`, `0x0A`, and the
  effect-specific selector at `0x11` outside known effects.
- Meaning of the fixed `0x38 0x9c` parameter pair in the `cc:83` all-key
  per-LED brightness command.
- Whether an undocumented full per-key RGB read-back exists below the Darfon
  managed API.
- Behaviour and argument model of `cc:85:ae`.
- Meaning of startup-only `cc:76`, `cc:99`, and `cc:9c`.
- Cause of the upper alias-bank holes for Backslash and Right Alt.
