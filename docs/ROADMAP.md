<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Luminate roadmap

This document describes the long-term direction of Luminate and the goals for
the first stable release. It focuses on project direction rather than
implementation details. Short-lived engineering notes are kept separately from
this durable project direction. The consolidated list of unfinished work lives
in [`TODO.md`](TODO.md).

## Current status

Luminate is in late pre-1.0 development. The core daemon architecture is now
largely established, and work is focused on validating that architecture through
broader hardware support, refining the public API, improving documentation, and
preparing for the first stable release.

Major architectural redesigns are no longer expected, though implementation
details keep evolving wherever experience with real hardware suggests something
better.

## Design philosophy

Luminate is guided by a small number of architectural principles.

**One owner.** Hardware should be owned by exactly one daemon, and applications
talk to the daemon rather than to individual devices.

**Stable client API.** Applications target `libluminate`. The public client API
is intended to become stable; internal implementation details, including the
plugin interface, are not.

**Explicit capabilities.** Hardware should expose the capabilities it actually
has. The API should avoid flattening fundamentally different hardware into an
artificial lowest common denominator.

**Upstream-first hardware support.** Hardware support belongs in the main
project. Plugins are expected to be maintained in-tree, or developed alongside
Luminate with the intention of being submitted upstream. Luminate intentionally
does not promise a stable external plugin ABI.

**Unknown means unknown.** The daemon should never invent hardware state.
Unknown state is represented explicitly until observations allow it to be
reconciled.

## Scope

Luminate provides a daemon-based lighting control system with:

- a stable client API (`libluminate`)
- a command-line interface
- companion user interfaces
- hardware plugins
- persistent intended state
- explicit observation and reconciliation
- plugin-based hardware support

A user should be able to install Luminate, discover supported hardware, control
it consistently, restart the daemon, and recover intended state.

## Core capabilities

Luminate aims to provide:

- Central daemon ownership of hardware
- Stable client API
- Device discovery
- Consistent topology
- Colour and brightness control
- Hardware effects where supported
- Persistence
- Safe handling of unknown hardware state
- Plugin isolation
- Deterministic ownership

## Plugin model

Plugins are implementation components rather than externally versioned
extensions. The preferred development model is upstream-first: prototype plugins
may be developed externally, but long-term maintenance is expected to happen
within the main Luminate project. The plugin ABI is intentionally free to
evolve.

## Control surfaces

The primary supported interfaces are `libluminate`, the CLI, and D-Bus.
Companion interfaces include a terminal UI and a graphical UI. All of these
remain ordinary clients of `libluminate` rather than special privileged
components.

## Hardware support

Initial stable releases focus on practical, commonly-used hardware rather than
strict parity with existing RGB ecosystems. Priority areas include:

- Alienware
- LIFX
- Philips Hue
- Linux LED class
- WLED
- Govee
- Razer
- Logitech
- Keychron
- OpenRGB compatibility

Future work may expand into additional IoT lighting ecosystems.

## Packaging

Supported deployment includes:

- systemd-based Linux distributions
- Alpine Linux with OpenRC
- musl builds
- Windows support
- macOS support
- package-manager installation
- local development workflows

Platform-specific integration should stay separate from daemon architecture.

## Quality goals

Before the first stable release, Luminate aims to provide:

- Stable public API
- Comprehensive documentation
- Useful diagnostics
- Reliable automated testing
- Robust behaviour across supported hardware

## Release criteria

Luminate 1.0 is expected once:

- The public API is stable
- High-framerate streaming is complete
- Documentation is production-ready
- Supported hardware has undergone substantial real-world testing
- Core hardware plugins are considered mature
- Packaging and deployment are well-supported
- Everything has been thoroughly tested and works reliably

## Future directions

Areas of interest beyond the initial release include Matter, HomeKit, Home
Assistant, and additional lighting ecosystems. These are areas of exploration
rather than commitments.

## Non-goals

Luminate does not currently aim to:

- Provide a stable third-party plugin ABI
- Support every RGB device before 1.0
- Flatten vendor-specific capabilities into generic abstractions
- Expose daemon internals as public API
