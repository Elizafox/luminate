<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Alienware plugin development

`luminate-plugin-alienware` is one plugin crate with two internal HID drivers:

- `0d62:d2b1` Darfon/Alienware internal keyboard, using `0xcc` reports.
- `187c:0550` and `187c:0551` AW-ELC controllers, using 33-byte `0x03` feature
  reports with a hidapi placeholder byte. A matching USB ID is only a
  discovery candidate; a read-only configuration query must resolve an exact
  platform profile before the device is published.

The plugin reports whichever devices are visible during HID enumeration. It can
often enumerate as a normal user, but applying updates usually requires root or
read/write access to the matching `/dev/hidraw*` node.

## Local permission workaround

For quick local testing, identify the hidraw node:

```sh
for node in /dev/hidraw*; do
  udevadm info --query=property --name="$node" | grep -E 'DEVNAME=|HID_ID='
done
```

Look for:

- `0003:00000D62:0000D2B1` for the keyboard.
- `0003:0000187C:00000550` for the imported AW-ELC profiles.
- `0003:0000187C:00000551` for AW-ELC.

Then grant the current user temporary access:

```sh
sudo setfacl -m "u:$USER:rw" /dev/hidrawN
```

Replace `hidrawN` with the node found above. This ACL is not persistent across
replug/reboot. Packaging should eventually install a udev rule that assigns the
matching hidraw nodes to the daemon user/group instead.

## Current scope

The first implementation intentionally covers a small, testable slice:

- Keyboard: whole-keyboard static/off, brightness finalizer, fixed-cadence
  firmware animations, per-key static colours, and volatile full-frame
  streaming on the recognized M16 R2 US-ANSI surface. Firmware animations are
  invoked as hardware effects: `aw-keyboard-breathe`, `aw-keyboard-pulse`,
  `aw-keyboard-spectrum`, `aw-keyboard-rainbow`, and `aw-sweeper` (the built-in
  left-to-right sweep). Their cadence is not caller-controlled, so advertising
  typed portable effects would be a small lie with a very confident API. The
  device publishes `shape:keyboard` and `form:laptop`; the recognized M16 R2
  US-ANSI layout also publishes `layout:us-ansi`.
- AW-ELC: live static/off, portable `morph`, and fixed-cadence hardware effects
  `aw-elc-breathe`, `aw-elc-pulse`, `aw-elc-spectrum`, and `aw-elc-rainbow`.
  On the m16 R2 these control trackpad/rear-logo zones; static/off writes use a
  separate persistent path for the power-button slots. Imported profiles
  expose only volatile live zones and do not advertise appearance slots or
  `SaveCurrent`.
  The trackpad ring publishes `shape:ring` and `form:trackpad-surround`. Both
  alien-head surfaces publish `shape:logo` and
  `alienware:shape/alien-head`, together with `form:laptop-lid` for the rear
  logo or `form:power-button` for the power button.

For example, invoke a fixed keyboard animation through the CLI with:

```sh
luminatectl set-effect --device alienware-keyboard --effect aw-keyboard-breathe --rgb '#0080ff'
```

Reliable rendered-state readback remains deferred. The plugin has no readback
callback and recommends `Restore`. Until the hardware has an exact query path,
successful writes are reported as assumed.

### AW-ELC platform profiles

The controller configuration query resolves these exact profiles:

| USB ID | Platform | Model | Raw zones | Validation |
| ------ | -------- | ----- | --------: | ---------- |
| `187c:0551` | `0x1102` | Alienware m16 R2 | 5 | Hardware-validated in Luminate |
| `187c:0550` | `0x0c01` | Dell G5 SE 5505 | 4 | OpenRGB-corroborated |
| `187c:0550` | `0x0a01` | Dell G7 15 7500 | 16 | OpenRGB-corroborated |
| `187c:0550` | `0x0e03` | Dell G15 5511 | 4 | OpenRGB-corroborated |
| `187c:0550` | `0x0e0a` | Dell G15 5530 | 4 | OpenRGB-corroborated |

The four-zone imported profiles publish opaque `left`, `middle`, `right`, and
`numpad` surfaces. The G7 profile publishes four keyboard regions followed by
`light-bar-1` through `light-bar-12`. These names describe logical firmware
zones, not keys or measured physical geometry. Imported descriptors carry a
visible validation warning and cite OpenRGB revision
`5e6d627f519a791487e766aeee51eed7bf7129ef`.

Unknown platform IDs, malformed replies, unexpected raw zone counts, and more
than one candidate controller fail closed. The plugin logs a diagnostic and
withholds AW-ELC topology instead of guessing. When a known imported profile is
resolved, it also emits a rate-limited warning that Luminate hardware
validation is still outstanding.

To submit a privacy-conscious identity report, enable info logging for this
plug-in, trigger a rescan, and copy only the `AW-ELC controller identity
resolved` line:

```sh
RUST_LOG=luminate_plugin_alienware=info luminated 2>&1 \
  | grep -F 'AW-ELC controller identity resolved'
```

That line contains only VID/PID, platform ID, raw zone count, and resolved
model. It does not contain the HID path, serial number, username, or unrelated
USB inventory. If resolution fails, include the single adjacent `AW-ELC
controller identity unavailable` diagnostic instead. Do not attach a complete
daemon log unless you have reviewed it for unrelated information.

### Firmware persistence (`SaveCurrent`)

The keyboard controller has a genuine explicit-commit persistence command,
`cc:84:03:00` (protocol spec §7), matching AWCC's own
`SaveCurrentEffectsToLastEffect()`. It takes no colour/effect payload. It
just commits whatever the keyboard is already rendering to non-volatile
firmware storage, so the plugin sends it as-is whenever `SaveCurrent` targets
the whole keyboard (device, `keyboard` surface, or `group:all`). This was
confirmed experimentally: program a Custom Colour effect, send `cc:84:03:00`,
power the system fully off, and the effect survives reboot. `SaveCurrent`
against a per-key `Element` target is still rejected because there is no per-key
variant of this command, only a whole-effect commit.

AW-ELC's power-button slots (`surface:power-button`) never needed
`SaveCurrent`: every write already lands directly in a persistent firmware
slot (see "Batched updates" below), which is why that surface advertises
`explicit_commit: false`. `SaveCurrent` against `surface:power-button` alone
is rejected as unneeded rather than silently no-op'd.

AW-ELC's live zones (`trackpad-ring`, `rear-logo`, `group:live-zones`) also
implement `SaveCurrent`, replaying the saved-animation transaction the
protocol spec documents (§6.2, §11): `REMOVE` → `START_NEW` →
(`SELECT_ZONES` → `ADD_ACTION` per zone) → `FINISH_N_SAVE` → `SET_DEFAULT`
against slot `0x0061`, with the same settling delay around the slot
open/close that real AWCC traffic also uses. There is no hardware readback
for the live animation state, so the plugin keeps an in-process shadow of the
action records each zone was last driven with (mirroring the keyboard's
per-key shadow) and replays that shadow into the saved slot; `SaveCurrent`
against a zone that has never been written live in this process fails rather
than guessing. Two zones with independently
different live content are saved correctly, each selected and written in its
own `SELECT_ZONES`/`ADD_ACTION` step inside one `START_NEW`/`FINISH_N_SAVE`
bracket. This was the open question blocking the implementation, and it is
now resolved and hardware-confirmed: an independent reference
implementation exercised this exact transaction shape on real hardware and
confirmed slot `0x0061` survives a full reboot, auto-restores at boot, and
replays via `USER_ANIM PLAY` (subcommand `0x0005`). A mixed target that
includes the power button alongside a live zone is rejected the same way
ordinary AW-ELC mutations already reject that mix.

The keyboard protocol notes currently describe the tested Alienware m16 R2
US-ANSI layout. Per-key/custom-colour support must not assume that key
indices map contiguously or identically across regional layouts, so the
per-key name table (§8 of the protocol spec, 85 physical keys) lives entirely
inside `keyboard_layout.rs`, scoped to `KeyboardLayoutId::M16R2UsAnsi`.
Nothing in `protocol/keyboard.rs` or `topology.rs` branches on US-ANSI
specifically. Both go through `KeyboardLayoutId::key_map()` /
`KeyboardLayoutId::physical_tags()` /
`KeyboardKeyMap::index_for_name()`/`named_positions()` generically, so a
second layout is additive (a new `KeyboardLayoutId` variant plus its own
table and physical metadata) rather than a rewrite.

The plugin has an internal layout scaffold:

- known layout ID: `m16-r2-us-ansi`, with its 85-key name table
- unknown layout identity fallback
- US-ANSI presence mask excluding documented reserved matrix positions
- `cc:93` layout identity readback helper

### Per-key custom-colour path (`cc:8c`/`cc:8b`)

Per-key colour is implemented following the documented sequence (protocol
spec §5): reset (`cc:8c:10`) → static slot config (`cc:8c:01`) → assignment
table (`cc:8c:05`/`06`/`07`) → per-key RGB table (`cc:8c:02`, chunked 15
records per report) → finalize (`cc:8c:13`) → commit (`cc:8b:01:ff`). There
is no per-key hardware readback (§6.2). A partial mutation is therefore safe
only after a successful complete frame has established every named key's
colour. Until then, element updates are rejected without opening the device;
unknown sibling colours are never replaced with off/black placeholders.

Without readback, `protocol/keyboard.rs` keeps an in-process shadow
(`KEY_COLOURS: Mutex<BTreeMap<u8, Rgb>>`) of each key's last applied colour,
keyed by canonical matrix index. A per-key mutation builds a candidate table,
writes it to hardware, and commits the shadow only after the HID write
succeeds. An uncertain write failure invalidates the complete shadow.

Whole-keyboard colour, effect, and clear writes invalidate the shadow after the
built-in effect report succeeds. Discovery identity changes also invalidate
it. The next per-key mutation is rejected until another complete frame is
written. Brightness writes leave the shadow intact. The shadow lasts for one
plugin-host lifetime; a restarted host begins with unknown per-key state.

Per-key targets only support a static colour (static `SetEffect` or
`SetEffect::{Static, Off}`/`Clear`); brightness and animated effects on an
`Element` target are rejected, since neither is meaningful for a single key on
this hardware path.

Hardware-validated on 2026-07-11, including two real bugs found and fixed
during validation (diagnosed against a known-working independent Rust
implementation, not just re-reading the spec):

- The exact `cc:93` tuple for the tested unit was captured from a live
  `alienware_probe` trace (`hardware_variant=[0x17,0x11]`, `layout=0x21`,
  `chassis_colour=0x00`) and hardcoded as the `M16R2UsAnsi` match arm in
  `KeyboardIdentity::layout_id()`, so per-key topology now actually appears
  against real hardware.
- **Off-by-one between the RGB table and the assignment table.** `cc:8c:02`'s
  `<key_index>` byte and the `cc:8c:05/06/07` assignment table's array
  position are different domains. The assignment position is
  `key_index - 1`, not `key_index`. The unshifted version produced exactly
  the flaky, key-dependent symptom seen on hardware: a key only lit if some
  other key at `index - 1` had already been touched in the same session
  (its own off-by-one artifact coincidentally satisfying its successor's
  requirement). Fixed in `keyboard_custom::apply_custom_colours`.
- Added the missing "clear known firmware slots" step
  (`cc:8c:01:<slot>` for `[0x01, 0x02, 0x05, 0x08, 0x09, 0x0e]`, right after
  reset) that a known-working reference sends and this plugin didn't.

A lightweight recolour path (`keyboard_custom::update_keys_colour`) sends
only `cc:8c:02` record(s) with none of the reset/slot-clear/assignment-table/
finalize/commit steps, confirmed on hardware not to cause the whole-board
flicker the full sequence does. `protocol::keyboard::apply_per_key` uses it only
when a complete frame shadow proves that every named key is already in the
firmware's assignment table.

Hardware follow-up on 2026-07-11 confirmed that separate per-key recolours can
use this lightweight path after the complete assignment table is established.

### Volatile frame streaming

The recognized `m16-r2-us-ansi` keyboard surface advertises full-frame uploads
at a conservative maximum of 15 frames per second. Each frame contains 85 RGB
pixels in the surface element order published by topology. Unknown keyboard
layouts do not advertise the capability, since applying a known layout's frame
order to uncertain hardware would light the wrong keys.

The first frame after a shadow miss initializes the complete volatile custom
assignment table. Later full frames are diffed against the per-key shadow and
send only changed `cc:8c:02` RGB records; an unchanged frame sends no HID
report. A frame may span several reports and is therefore advertised as
immediate but non-atomic.

Frame uploads never use `cc:84:03:00`, so they do not persist an effect to
firmware. They also remain hardware-only at the daemon layer: streamed frames
are not added to desired state or replayed after restart. See the
[`libluminate` Bad Apple example](../bad-apple-keyboard-demo.md) for a complete
client.

### Batched updates

Daemon startup replay pushes persisted state to plugins as a batch when
possible (`luminate_plugin_api::PluginUpdateBatch` /
`PluginDescriptor::apply_batch_cbor`, opt-in and best-effort; see
[Batched updates](../plugin-authoring.md#batched-updates) for the ABI contract).
This plugin implements it: keyboard per-key `Element` targets in a batch are
resolved independently, staged under one lock, and applied with exactly one
hardware transaction for the whole group (the full rebuild if any key is newly
assigned, otherwise the lightweight recolour path). A full rebuild primes the
assignment table for the detected layout's named keys, with unspecified keys
off/black, so subsequent single-key updates do not need another full rebuild
unless a whole-keyboard effect invalidates the table. The staged shadow is
committed only after that shared hardware transaction succeeds, turning
"N persisted keys → N flickers at startup" into one transaction without
recording a false success if the HID write fails.

The AW-ELC power-button alien head is one `power-button` surface with `ac` and
`battery` appearance slots. One `SetAppearanceSlots` operation reaches the
plugin as a logical group. Firmware charging slot `0x5d` is a derived
two-keyframe morph between those base colours, so a complete update resolves
both endpoints and writes `0x5d` once. A later partial update can rebuild the
morph from the in-process shadow, but if the other endpoint is still unknown
after plugin startup the update is rejected before writing rather than
replacing persistent firmware state with a guessed colour.

This intentionally replaces the former `power-button-ac` and
`power-button-battery` surface IDs. Persistence format 12 migrates their
desired appearances, adopted baseline, and scene bindings into the new slots.
It preserves the original state file as
`state.json.pre-appearance-slots-v12.bak` before the next persisted rewrite.
Clients that stored the old topology IDs must discover and use
`surface:power-button` with slot IDs `ac` and `battery`.

Imported AW-ELC live entries with identical lowered action records are
coalesced into one bounded multi-zone animation transaction. Distinct effects
remain distinct transactions. Shadows include the exact controller and
platform identity and are invalidated when discovery changes or the controller
disappears. The m16 R2 stays on its established individual live-zone and
power-slot paths, so a power-button operation can never enter generic live
batching.

Hardware validation on 2026-07-11 confirmed single-zone live selection is
independent for the tested controller:

- baseline: `trackpad-ring` static green, `rear-logo` static blue, keyboard
  scanner still running
- `trackpad-ring` spectrum changed only the ring; rear head stayed blue
- `rear-logo` rainbow changed only the rear head; ring continued spectrum and
  keyboard continued scanner

## Hardware validation log

Validated on 2026-07-10 with manual `setfacl` access to the relevant hidraw
nodes:

- Probe detected both controllers:
  - keyboard `0d62:d2b1`
  - AW-ELC `187c:0551`
- Topology listed both `alienware-keyboard` and `alienware-aw-elc`.
- Keyboard whole-device effects worked:
  - static red
  - breathe
  - spectrum
  - rainbow wave (confirmed slow-looking, matching the known built-in path)
  - scanner / left-to-right sweep
- AW-ELC live zone writes worked:
  - `trackpad-ring` static green
  - `rear-logo` static blue
  - `trackpad-ring` independent spectrum while `rear-logo` stayed static blue
  - `rear-logo` independent rainbow while `trackpad-ring` continued spectrum
  - `live-zones` group static purple
  - `live-zones` off
  - `live-zones` breathe
  - `live-zones` spectrum
- AW-ELC power-button writes worked while the machine was on battery:
  - `power-button` static dim amber
  - `power-button` off

During validation, the first AW-ELC attempt accepted HID reports but displayed
the wrong state. The cause was mixed wire-buffer and hidapi-buffer offsets in
the live animation envelope and `03:24` action-record builders. Byte-level unit
tests now pin the documented packet prefixes:

- `00 03 21 00 01 ff ff 00 00`
- `00 03 23 01 00 01 00`
- `00 03 24 00 07 d0 00 fa ff 00 00`

Validated on 2026-07-15, keyboard and AW-ELC firmware persistence:

- Keyboard `SaveCurrent` (`cc:84:03:00`): programmed a Custom Colour effect,
  sent the persist command, fully powered the system off, and confirmed the
  effect survived reboot.
- AW-ELC live-zone `SaveCurrent` (saved-animation slot `0x0061`): applied
  independently different live effects to `trackpad-ring` and `rear-logo`,
  saved them via the `REMOVE`/`START_NEW`/`SELECT_ZONES`/`ADD_ACTION`/
  `FINISH_N_SAVE`/`SET_DEFAULT` transaction, and confirmed the saved slot
  survives a full reboot, auto-restores at boot, and replays on demand via
  `USER_ANIM PLAY`. This resolved the previously open question of what a
  single saved slot means for two zones with different live content: each
  zone is selected and written independently inside one save transaction, and
  the firmware preserves both.
- Confirmed against the same independent reference Rust implementation of
  this protocol used throughout hardware validation (see the per-key
  off-by-one fix above, also caught this way).
