<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Appearance slots

An appearance slot is a named appearance or effect program stored by firmware
and selected by a condition outside Luminate's direct control. A slot belongs
to one physical surface; it is not a child target and has no independent
physical identity, brightness, emission, geometry, elements, or power domain.

Slot IDs are stable machine identifiers within their surface. Display names
need not be stable. IDs and names must be non-empty and unique within the
surface. Descriptors carry appearance and persistence capabilities for each
slot, so different slots on one surface may accept different effects. Keyframes
remain part of an effect program rather than becoming slots themselves.

Appearance slots are valid only on surfaces. Device, group, and element
capabilities must not advertise them. A surface may also expose ordinary
brightness, emission, readback, and power-domain capabilities, but it should
advertise an ordinary unqualified appearance only when the hardware truthfully
supports both an immediate appearance and stored slotted programs.

## Grouped updates and completeness

`SetAppearanceSlots` carries an ordered collection of slot values for one
concrete surface. It is a single logical mutation and must not be expanded into
sequential `SetEffect` operations. The daemon validates the whole mutation
before the first hardware write, then sends one operation to the plugin. This
grouping does not claim that the underlying hardware transaction is physically
atomic.

The update policy advertised by the surface determines which requests are
safe:

- `Independent` allows any advertised slot to be updated alone.
- `PartialIfKnown` allows a partial client request only when every omitted
  coupled slot is known. The daemon fills the omitted values and submits a
  complete plugin operation.
- `CompleteSet` requires every advertised slot in each request.

An update is invalid when it is empty, repeats a slot ID, names an unknown slot,
uses an unsupported effect, or omits a value required by its policy. The plugin
remains the final authority because its in-process hardware shadow can become
unknown independently of daemon desired state. It must reject an unsafe partial
update rather than guess a missing value.

## State, observations, and scenes

Desired and adopted slot state is stored under the owning surface, keyed by
slot ID. Slots do not receive manufactured `TargetId`s. Known values are
tracked independently so a partial observation does not erase uncertainty
about other slots.

An observed appearance-slots facet reports the known values and whether the
observation is complete. The aggregate appearance facet continues to describe
the visible surface; multiple stored programs are not flattened into `Mixed`.

A scene binding may carry an ordered slot-value collection. Validation uses the
same duplicate, capability, and completeness rules as a direct mutation, and
application emits one grouped operation per surface. Capture includes only
known slot values and fails when the advertised policy requires a complete set
that cannot be constructed safely.

## Programmable sequences

Each slot contains an existing `Effect`, so a descriptor can enable a portable
or hardware effect without another slot API change. Arbitrary keyframes with
individual durations remain a separate portable-effect design. They should be
added only for validated hardware, with explicit frame-count, duration,
looping, direction, and interpolation constraints; slot support alone does not
promise that firmware accepts arbitrary intermediate frames or timings.

## Alienware AW-ELC

The first appearance-slotted surface is the AW-ELC power-button alien head.
Its `ac` and `battery` base-colour slots share a `PartialIfKnown` update because
firmware derives its charging morph from both values. The hardware-specific
record layout, update transaction, and safety constraints are documented in
[the AW-ELC protocol specification](../../design/hardware/alienware/alienware-aw-elc-rgb-hid-protocol-spec.md).
