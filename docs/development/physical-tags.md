<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Physical tags

Physical tags are open, ordered sets of semantic presentation hints attached
to devices, surfaces, and elements. Device tags describe the physical fixture
or peripheral as a whole. Surface tags describe only one addressable region,
installation, or mapping. Element tags describe one individually addressable
physical object within a surface. A tag's meaning is scope-neutral: attach it
at the narrowest scope where the fact is true.

Tags supplement `DeviceCategory` and logical `SurfaceKind`; they do not replace
either, advertise capabilities, authorize operations, or select protocol
behaviour. Their absence means that the physical form is unknown. Clients must
therefore retain a sensible generic presentation when a tag is absent or
unknown.

## Standard tags

The following vendor-neutral tags are the standard vocabulary understood
across Luminate. This table is not an exhaustive allow-list: plugins may
publish additional tags without adding them to this document. Clients must
preserve unknown tags and ignore them safely.

| Tag | Meaning |
| --- | --- |
| `shape:keyboard` | A physical keyboard, whether integrated into another device or housed separately. |
| `shape:ring` | A closed, ring-shaped illuminated region, which may surround another physical region. |
| `shape:logo` | An illuminated emblem or logo. A provider-specific tag may identify its exact outline. |
| `shape:flexible-strip` | A flexible, elongated lighting assembly intended to bend around or along an installation. |
| `shape:modular-light-bar` | A lighting assembly made from rigid bar modules that can be joined into an arrangement. |
| `shape:cylinder` | A cylindrical fixture or addressable region. This does not imply how elements wrap around it. |
| `shape:a19` | A bulb with the standard A19 envelope form. |
| `shape:br30` | A bulged-reflector bulb with the BR30 envelope form. |
| `shape:gu10` | A lamp with the recognizable GU10 spotlight form and base. |
| `shape:par38` | A parabolic aluminized reflector lamp with the PAR38 envelope form. |
| `shape:downlight` | A fixture intended to cast light downwards from a ceiling or similar mounting surface. |
| `shape:candle` | A candle-shaped bulb or fixture; this does not imply a matrix or flame-effect topology. |
| `layout:us-ansi` | A keyboard with the United States ANSI key arrangement. |
| `layout:wrapped-horizontal` | Successive horizontal rows wrap around the tagged surface rather than occupying one flat plane. |
| `form:standalone` | The tagged object has its own enclosure rather than being integrated into a larger host device. |
| `form:laptop` | The tagged keyboard is integrated into a laptop chassis. |
| `form:split` | The tagged keyboard is divided into physically separate typing sections. This may accompany `form:standalone`. |
| `form:trackpad-surround` | The tagged region surrounds a trackpad. |
| `form:laptop-lid` | The tagged region is integrated into a laptop lid. |
| `form:power-button` | The tagged region is integrated into a physical power button. |

## Plugin-defined tags

Plugins may support physical tags beyond the standard vocabulary. Prefer a
standard vendor-neutral tag whenever its meaning fits. Document other tags
with the plugin that provides them. Namespace genuinely provider-specific
forms to avoid collisions, for example `govee:shape/hexagonal-tile`.

A client may recognize plugin-defined tags, but it must not require every tag
it understands to appear in this document. Likewise, inclusion in this
document does not require every plugin or client to use or specially render a
tag.

## Publishing and consuming tags

Publish physical form only from authoritative identity data, a hardware
profile, or explicit installation configuration. Do not infer it from a
user-controlled label or a loose model-name match. For example, a linear
addressable surface does not by itself establish that the device is a flexible
strip.

Do not copy a device or surface tag onto its descendants merely for consumer
convenience. Tags are not inherited. A device, surface, and element may
independently carry the same tag when the fact is genuinely true at each scope.
Multiple tags are appropriate when they express independent facts, such as
`shape:cylinder` and
`layout:wrapped-horizontal` on one surface.

Tags in one namespace are not necessarily mutually exclusive. Describe
independent facts separately instead of combining them into a progressively
more specific tag. For example, a split US-ANSI keyboard can publish
`shape:keyboard`, `layout:us-ansi`, `form:standalone`, and `form:split` so that
clients can recognize each fact independently.

The daemon preserves provider order. Within a device, surface, or element, a
tag must be non-empty, have no surrounding whitespace, and occur only once.

## Verification scope

Physical tags are provider-declared metadata. Their transport, validation,
scope, ordering, and public projections are covered by deterministic tests.
Adding element-level transport does not change hardware discovery or control,
and introduced no new production hardware claims, so it requires no physical
hardware validation.
