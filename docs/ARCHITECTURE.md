<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Luminate architecture

This document describes the architectural principles behind Luminate. Rather
than documenting implementation details, it explains the ideas that shape the
project. They matter more than any particular module because the implementation is
expected to keep evolving while these goals stay put.

Current direction and release goals live in the [roadmap](ROADMAP.md).

## Overview

Luminate is a daemon-based lighting control and state management system.
Applications talk to a single daemon that owns hardware access, persistent
state, reconciliation, and coordination. Hardware support comes from plugins,
while applications interact with the daemon through stable client interfaces
such as `libluminate` and D-Bus.

The optional D-Bus companion is a policy-enforcing front end, not a second
source of truth. It projects complete daemon models through versioned native
interfaces, forwards dirty events, and refetches authoritative state through
libluminate. See the [D-Bus consumer guide](development/dbus.md) for the public
contract and the [parity matrix](development/dbus-parity.md) for deliberate
transport exceptions.

Lighting looks simple, but it is fundamentally a distributed systems problem.
Many devices cannot reliably report their current state, different hardware
exposes wildly different capabilities, and several applications may want to
observe or control the same hardware at once. Luminate is therefore designed
around explicit state management rather than direct command forwarding.

## Core architectural principles

### One daemon owns the hardware

Hardware is owned by exactly one running daemon. Applications never talk to
devices directly; they talk to Luminate, which coordinates ownership,
persistence, reconciliation, streaming, and plugin interaction. That gives a
single authoritative control plane and stops competing applications from
independently trying to manage the same hardware.

Luminate may communicate with other daemons, but it treats its own information
as authoritative.

### Control is state-driven

Clients express the state they want hardware to reach rather than issuing
imperative commands. Events exist for observation and notification, but they are
not the source of truth: the daemon owns desired state and continuously
reconciles hardware towards it whenever that is appropriate.

### Desired state and observed state are different

One of the central ideas in Luminate is that desired state and observed state
are independent concepts. Desired state represents what the user intends;
observed state represents what hardware is believed to be doing.

These differ constantly. Hardware may be disconnected, commands may fail, some
devices never report their state, and some silently ignore operations they don't
support. Keeping the two concepts separate lets Luminate behave predictably even
when hardware cannot.

### Unknown is a valid state

Luminate does not guess, because guessing leads to gaslighting the user. If
hardware state cannot be determined, it is represented as unknown. It is better
to be unsure than to be confidently wrong.

Plugins decide what can be observed reliably for a particular device. A device
that cannot provide trustworthy information should have that uncertainty
exposed rather than papered over with invented answers.

Aggregate state follows the same rule. Luminate computes appearance from the
bottom up through surfaces, groups, devices, and collections. Constituents
that visibly disagree produce `mixed`; incomplete knowledge stays unknown.
A successful aggregate write updates the known state of the constituents it
covers, and a later constituent change recomputes every ancestor. This avoids
showing a stale, reassuringly uniform value after the underlying lights have
diverged.

### Persist intent, not reality

Luminate persists intended state and does not try to persist assumptions about
physical hardware. Many devices cannot report their actual state, particularly
across daemon restarts or power cycles, so persisted assumptions would drift
from reality soon enough. Instead, Luminate remembers user intent and reconciles
hardware when policy permits.

### Plugins translate hardware

Plugins exist to translate between Luminate's internal model and hardware.

The daemon owns:

- desired state
- observed state
- persistence
- reconciliation
- ownership
- topology

Plugins own:

- hardware communication
- protocol translation
- device-specific quirks
- hardware discovery

Plugins should stay as stateless as is practical. Any internal state should be
ephemeral and exist only to support talking to hardware; durable system state
belongs in the daemon.

### Configuration has explicit authority layers

Administrator-owned bootstrap policy and daemon-managed desired configuration
have different trust and lifecycle boundaries. The daemon never rewrites global
policy. It persists runtime-safe management choices separately, then resolves
desired and effective values without discarding managed choices hidden by
global locks.

Plugin management commits durable intent before reconciling runtime state.
Consequently, an enabled plugin may truthfully remain failed while the daemon
retains the request for retry. See the
[plugin management architecture](development/architecture/plugin-management.md)
for the complete authority, inspection, and transaction model.

### Topology matters

Luminate deliberately models hardware topology. Devices contain surfaces,
surfaces contain elements, controllers may contain several independent devices,
and groups may span arbitrary hardware.

This hierarchy reflects how lighting hardware actually behaves and matches what
users expect: setting an entire keyboard naturally affects all of its keys,
while setting one zone affects only that zone. Flattening everything into a
generic list of RGB endpoints would throw away useful information.

### Capabilities over hardware classes

Luminate models capabilities rather than product categories. A keyboard is not
just "a keyboard": some keyboards support per-key RGB, while others expose only a
single backlight, some expose firmware effects, and some expose only
brightness. IoT lighting ranges just as widely, from simple on/off relays
through RGB strips to fully addressable matrices.

Rather than growing an ever-deeper hierarchy of hardware subclasses, Luminate
models what hardware can do. Vendor-specific capabilities remain possible
through extensible capability interfaces where that fits.

### Streaming is a distinct ownership mode

Frame streaming is a separate mode of operation. While a stream is active, one
client temporarily owns exclusive access to the streamed target.

Streaming is meant for animations, demonstrations, and other high-throughput
workloads. It is deliberately distinct from ordinary state mutation, and it
participates in the daemon's ownership and lifecycle management.

### Clients own presentation

Applications remain ordinary clients. The daemon owns control, reconciliation,
persistence, and coordination; clients own presentation, interaction, and user
experience.

Whether the client is a CLI, TUI, GUI, Home Assistant integration, or something
custom, it communicates through supported client APIs rather than daemon
internals. No client is privileged.

### Stable APIs, evolving internals

Luminate deliberately distinguishes public interfaces from implementation.
Stable interfaces are `libluminate`, D-Bus, and documented CLI behaviour.

Persistent intended-state snapshots are described in
[Daemon-managed scenes](development/architecture/scenes.md).
Internal interfaces are the plugin ABI, daemon internals, scheduling, the IPC
implementation, and internal protocols.

Internal interfaces stay free to evolve as hardware support expands and the
architecture matures.

### Upstream-first hardware support

Hardware plugins are expected to be maintained alongside Luminate. Prototype
plugins may live externally while hardware support is under active development,
but the long-term goal is upstream inclusion.

Luminate intentionally does not provide a stable third-party plugin ABI. That
lets the hardware abstraction evolve as new classes of device turn up with new
requirements, instead of permanently freezing architectural mistakes.

### Complexity belongs inside the system

Lighting hardware is surprisingly complicated. Different devices expose
different capabilities, many cannot report state, some silently ignore requests
they don't support, and some need startup synchronization. Luminate deliberately
absorbs that complexity.

Users should interact with concepts such as devices, colours, brightness,
effects, and groups. They should not need to understand reconciliation,
observation models, generation tracking, or capability facets to
control their lighting. The architecture should be as simple as possible for
users, but no simpler.

## Trust model

Luminate deliberately separates responsibility. Applications express intent, the
daemon owns authoritative desired state, and plugins determine what hardware can
reliably observe or apply.

Hardware itself is assumed to be neither truthful nor deceptive.
Device-specific behaviour is characterized by plugins, which expose only
information that can be responsibly represented.

## Summary

Nearly every design decision in Luminate follows from a small number of ideas:

- one daemon owns hardware
- control is state-driven
- desired and observed state are distinct
- unknown is preferable to incorrect
- topology and capabilities matter
- plugins translate hardware
- configuration authority is layered
- clients remain ordinary clients
- complexity belongs inside the daemon
- public APIs are stable; implementation remains free to evolve

These principles are expected to hold even as the implementation keeps changing.
