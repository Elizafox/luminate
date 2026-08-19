<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Razer BlackWidow V4 Pro RGB HID protocol

Status: experimental, partially hardware-validated

This document describes the original full-size US-ANSI Razer BlackWidow V4 Pro
(`1532:028d`) used to develop Luminate's native Razer plugin. Protocol facts
derived from OpenRazer use revision
`6820f9da169d354bc7e6e93a0aa8683a6bb75792`. Live observations were made on one
dedicated test unit with USB device revision `1.01` and protocol firmware
version `1.0`.

## Interface and transport

The composite USB device has five HID interfaces. Interface 3 has no HID
subclass or input protocol and is the verified lighting control interface.
Luminate selects the exact VID:PID and interface number; it never probes an
unknown Razer product or opens the keyboard, mouse, media, or vendor input
interfaces.

The common command is a 90-byte feature report. `hidapi` requires a leading
zero report-ID byte in its userspace buffer, making the send and receive buffers
91 bytes. Requests and responses on interface 3 work without detaching the
kernel input drivers. The verified transaction ID is `0x1f`. Luminate waits
600 μs after each feature-report send, matching OpenRazer's BlackWidow Chroma
timing class.

Every response-bearing command is accepted only after validating:

- the 90-byte length and leading HID report ID;
- protocol type and reserved byte;
- XOR CRC over report bytes 2 through 87;
- transaction ID, command class, and command ID;
- zero remaining packets; and
- a successful terminal status.

The keyboard returned `Busy` for the first tested brightness write. Retrying
the same idempotent command after 10 ms succeeded. Luminate therefore permits
at most three attempts for this transient status. Other status and I/O failures
remain distinct errors and are not optimistically treated as success.

## Verified commands

The following operations have passed deterministic byte tests and live command
tests:

| Operation | Class | Command | Data | Response |
| --- | --- | --- | --- | --- |
| Read firmware | `0x00` | `0x81` | two bytes | required |
| Read serial | `0x00` | `0x82` | 22 bytes | required |
| Set extended-matrix effect | `0x0f` | `0x02` | effect-specific | required |
| Read brightness | `0x0f` | `0x84` | three bytes | required |
| Set brightness | `0x0f` | `0x04` | three bytes | required |
| Activate custom mode | `0x0f` | `0x02` | 12 bytes | none |
| Upload custom row span | `0x0f` | `0x03` | declared length `0x47` | none |

Serial queries returned stable non-empty data twice in one session. Luminate's
test does not print or persist the serial. The USB descriptor itself has no
serial. Two independent close-and-reopen cycles returned the same protocol
identity, but power-cycle stability and multiple-identical-device behaviour
remain unresolved.

The extended effect arguments begin with storage `0x01` and backlight target
`0x05`. Verified effect IDs are off `0x00`, static `0x01`, and spectrum `0x03`.
Static uses nine argument bytes, with colour count `0x01` followed by RGB.
Off and spectrum use six argument bytes.

A deliberate visual sequence produced off, solid red, solid green, solid blue,
off, and spectrum in that order. Static-to-static changes transitioned
smoothly in firmware; off was immediate. This means firmware static commands
must not be described as frame-accurate instantaneous changes.

Both wave directions were observed moving the rainbow left-to-right and
right-to-left. Wheel directions rendered clockwise and anticlockwise. Reactive
mode illuminated a pressed Tab key red. Red and blue single-colour breathing
faded the selected colour to black. Dual red/blue breathing alternated red,
black, blue, black. Two-colour starlight rendered different colours on
individual keys. These observations validate the corresponding effect schemas;
unexercised random-colour forms remain hidden.

Brightness writes at both protocol endpoints, `0` and `255`, read back exactly.
The test restored the value observed before it began. Brightness remains a
whole-device control; no per-element brightness authority is claimed.

## Custom frames

OpenRazer declares an 8×23 matrix. A row-span upload carries two leading zero
bytes, row, inclusive start and stop columns, and packed RGB values. The largest
row has 23 colours (69 RGB bytes) after the five-byte prefix. The report's
declared data length remains `0x47`, even though the meaningful argument bytes
extend beyond that declared length. CRC still covers the complete fixed
argument area.

Custom-mode activation and row uploads deliberately do not read responses on
this model. Luminate still bounds rows to 0–7, columns to 0–22, rejects empty or
overflowing spans before opening hardware, and waits after each send. A full
eight-row cyan frame produced uniform cyan across all visible lighting, and
spectrum restoration completed. Physical occupancy and auxiliary-zone mapping
remain pending coordinate sweeps.

A first row-isolation sweep produced the following physical grouping:

| Matrix row | Observed physical scope |
| --- | --- |
| 0 | Top key row |
| 1 | Second key row |
| 2 | Third key row |
| 3 | Fourth key row |
| 4 | Fifth key row |
| 5 | Sixth key row |
| 6 | Keyboard side lighting |
| 7 | Wrist-rest lighting, provisional |

Rows 0–5 followed the expected physical keyboard rows. Row 6 controlled side
lighting. Row 7 appeared to control the wrist rest, but that identity remains
provisional. A detached-wrist-rest repeat produced no visible row 7 output,
supporting that identity without establishing attached element ordering.

A subsequent coordinate pass used OpenRGB commit
`bcfaa7e8a740a414b96cd65c397e3dfc` as an independent GPL-2.0-or-later
corroborating source. Its BlackWidow V4 Pro table describes six 23-column key
rows, two nine-element side-light strips, five unused cells, and a 20-element
wrist-rest strip. The table remains inherited evidence rather than a substitute
for testing on this unit.

Live tests with the wrist rest detached confirmed that `r0-c0` is unoccupied,
`r0-c1` controls the command dial, and `r0-c2` controls Escape. They also
confirmed the complete row 6 segmentation: `c0` through `c8` are the left side
from top to bottom, `c9` through `c17` are the right side from bottom to top,
and `c18` through `c22` are unoccupied. No row 7 light was visible while the
wrist rest was detached.

Luminate publishes the corroborated US-ANSI key map and the live-validated
side-light ordering as element identities. Row 7 columns 0 through 19 are
published as presumed wrist-rest elements from the OpenRGB table; columns 20
through 22 remain unoccupied. Each element update is applied through a complete
8×23 shadow frame established by a successful full-frame upload. Until that
frame is known, element updates are rejected rather than inventing black
sibling state. Firmware effects, topology refreshes, and uncertain transport
outcomes invalidate the shadow. Element-only batches update one candidate
shadow and issue one complete frame upload.

A 120-frame live transport test uploaded every row of every frame and measured
162.1 complete frames per second on the development host. Luminate advertises
a conservative maximum of 60 Hz. This establishes transport throughput, not
atomic presentation: each frame is still eight immediate row writes and the
device provides no acknowledgement for custom-mode or row-upload commands.

## Safety and remaining work

Physical disconnect and reconnect testing preserved the protocol identity.
Brightness written with OpenRazer's `VARSTORE` value (`0x01`) survived the
power cycle, while brightness written with `NOSTORE` (`0x00`) reverted to the
previous persistent value. Purple and white `VARSTORE` static effects returned
after reconnect. A later green `NOSTORE` static effect did not return; the
keyboard briefly displayed an older cyan custom frame during startup, then
selected the persisted spectrum effect. Ordinary Luminate updates now use
`NOSTORE`, while
an explicit `SaveCurrent` replays the current firmware effect and brightness
with `VARSTORE`. Custom frames cannot be saved. `VARSTORE` is the older
snapshot-style persistence mechanism. It is distinct from Razer's newer,
incompletely decoded selectable-profile protocol, so Luminate exposes a
current-state commit rather than profile enumeration or selection.

After a full Linux system suspend and resume, the control interface reopened,
the protocol identity query passed, and write-enabled plugin-host conformance
completed successfully through the volatile command path.

The packaged Linux rule grants the daemon access only to hidraw nodes for
`1532:028d` interface 3. Input reports are never read or logged. Persistence is
advertised only as an explicit current-state commit. Firmware updates, factory
modes, polling, input remapping, and all other non-lighting features are out of
scope.

Before this profile can be marked `Validated`, testing still needs to cover the
complete live key-map confirmation, attached wrist-rest ordering, and
concurrent normal input. Attached wrist-rest testing is currently unavailable.
Until then Luminate
exposes only the smaller capability set supported by completed evidence.
