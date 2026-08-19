<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Alienware legacy v2 HID protocol

This note records the first research slice for the nine-byte legacy AlienFX
protocol. It is not a hardware-validation record.

## M14x candidate

The initial exact candidate is USB `187c:0525`, with the stable Luminate device
ID `alienware-m14x`. The pinned `trackmastersteve/alienfx` reference is
revision `08ec685ae4c201d1157677dee92cd5fdd1beb279`, whose
`controller_m14xr3.py` identifies the controller as Alienware M14XR3 and lists
eleven masks. That source explicitly says the controller still needs the
correct zone codes, so Luminate keeps this table internal until a descriptor
capture and opt-in hardware test confirm it.

The pinned AlienFX-Tools survey commit is
`52713b238066d1343a492018ded546ff751cfcd4` (2026-07-28). It independently
classifies the family as a nine-byte, four-bit-per-channel HID output protocol.
Neither reference is a build input or a source of automatic profile data.

## Packet framing

Reports are nine bytes and begin with report ID `0x02`. The command byte is at
offset one. Static colour uses command `0x03`, with a one-byte block number,
three big-endian zone-mask bytes, and two packed colour bytes:

```text
02 03 block mask-hi mask-lo red4|green4 blue4|0000 00 00
```

The M14x masks fit within the low two bytes of the three-byte wire field. Four
bits are retained from each 8-bit RGB channel, so `0xff`, `0x80`, and `0x0f`
become `f`, `8`, and `0` respectively.

The minimal volatile static transaction is a status query (`0x06`), one or
more static-colour commands, loop-block end (`0x04`), and transmit/execute
(`0x05`). Luminate does not currently encode reset (`0x07`), saved-state (`0x09`),
power-state, brightness, blink, morph, or speed commands for this family.

Status reply framing, busy handling, required delays, the expected HID usage,
and the complete descriptor remain unknown pending a capture from the exact
controller. Discovery must therefore remain fail-closed until those facts are
recorded.

## Candidate logical masks

| Surface | Mask |
| --- | ---: |
| left-keyboard | `0x0001` |
| centre-left-keyboard | `0x0002` |
| centre-right-keyboard | `0x0004` |
| right-keyboard | `0x0008` |
| right-speaker | `0x0020` |
| left-speaker | `0x0040` |
| logo | `0x0100` |
| touchpad | `0x0200` |
| status-leds | `0x0800` |
| power-button | `0x2000` |
| hdd-leds | `0x4000` |

These are candidate firmware masks, not a claim that every surface is safely
writable. The eventual profile must cross-check them against the second
reference and an opt-in test of each advertised zone. No generic zone scan is
permitted.

## Validation procedure

Capture VID/PID, product/version strings, usage page and usage, report lengths,
and the report descriptor without serial numbers. Stop AWCC, OpenRGB,
AlienFX, and other controllers before writing. Test one non-power zone with
low-intensity red, green, blue, and off; restore it; then identify remaining
zones individually. Persistence, reset, power-state slots, and firmware
animation commands require separate authorization and recovery procedures.
