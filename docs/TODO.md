<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Luminate work list

This is the single summary of unfinished project work. `ROADMAP.md` describes
durable direction; the detailed `plans/*-plan.md` documents retain design decisions,
protocol evidence, and validation procedures. Update this file when work is
started, completed, deferred, or newly discovered instead of adding another
project-wide checklist.

Items described as blocked on hardware or another platform are implementation
ready but cannot be completed by ordinary hermetic CI alone.

## Before 1.0

- [ ] Stabilize the public client API.
- [ ] Complete and validate high-framerate streaming on real hardware,
      including sustained-rate and restoration behaviour.
- [ ] Finish production-ready user, operator, API, and plugin-authoring
      documentation.
- [ ] Profile daemon and streaming performance and address release-blocking
      findings.
- [ ] Expand real-world hardware testing enough to define the mature core
      plugin set and make support claims evidence-based.
- [ ] Finish supported-platform packaging: Windows MSI packaging and
      signed/notarized macOS distribution.
- [x] Investigate and fix iceoryx2 cleanup warnings when running the tests.
- [ ] Add a rate-limiting system for clients.

### HTTP LAN security

The HTTP API is safe for direct LAN exposure when deployed according to the
[HTTP API operator guidance](development/http-api.md). The completed security
work includes:

- [x] Add native TLS to `luminate-http`; require it for non-loopback listeners
      unless an explicitly named insecure-development option is selected, and
      prevent configuration mistakes from silently downgrading to plaintext.
- [x] Validate certificate/key pairs before binding, reload them atomically
      while retaining the last valid pair, keep private keys in protected
      storage, and avoid exposing key material in diagnostics.
- [x] Complete the certificate lifecycle: validate identity and validity
      periods; warn before expiry; and document trust, issuance, renewal,
      atomic replacement, rollback, and revocation workflows.
- [x] Bound simultaneous connections, TLS handshakes, request bodies, request
      rates by TCP peer, and companion WebSocket ticket capacity.
- [x] Complete the resource limits for headers, slow and idle clients,
      unauthenticated health/OpenAPI traffic, WebSocket upgrades, sessions,
      messages, queues, and liveness.
- [x] Define the browser security model: use a deny-by-default CORS policy,
      validate WebSocket `Origin`, and document safe bearer-token storage so a
      hostile LAN site cannot drive authenticated requests through a browser.
- [x] Derive peers from the TCP connection rather than forwarding headers;
      require explicitly configured proxy addresses and a dedicated credential
      for trusted-proxy mode; and require TLS on a non-loopback proxy hop.
- [x] Strip untrusted forwarding headers at the outer middleware boundary so a
      future handler cannot accidentally consume them.
- [x] Keep bearer secrets out of ordinary responses and traces, and exchange
      them for bounded, endpoint-specific, one-use WebSocket tickets whose
      query strings are removed before tracing.
- [x] Complete the bearer replay and disclosure review, including rotation,
      expiry, revocation, audit, metrics, errors, and crash reports.
- [x] Use a distinct, revocable daemon token as the direct-client boundary and
      derive its canonical authority, subject, groups, and credential ID from
      daemon authentication rather than caller-selected identity headers.
- [x] Document least-privilege per-user or per-device token bootstrap,
      rotation, revocation, and recovery. Do not present a shared household or
      front-end token as individual identity.
- [x] Separate ordinary LAN bearer authentication from trusted identity
      delegation. A network client must not be able to select an authority,
      subject, or verified groups merely by adding a header; expose delegation
      only on a separately authenticated front-end channel, or disable it on
      the direct-client listener.
- [x] Require every delegated operation to be allowed for both the asserted end
      user and an immutable front-end actor ceiling, and preserve that ceiling
      and its revocation through the attestation path.
- [x] Expand the delegated-authorization allow/deny integration matrix and
      verify that event-ticket renewal cannot lose the front-end ceiling.
- [x] Preserve both identities through authorization and auditing: the remote
      subject and credential ID, plus the delegating front-end principal and
      credential ID when present. Revocation, ownership, quotas, rate limits,
      and incident investigation must use the intended identity rather than
      collapsing all LAN clients into the `luminate-http` service account.
- [x] Rate-limit failed authentication by bounded network and credential
      dimensions without trusting spoofable forwarding headers. Keep failures
      uniform enough to avoid token, subject, authority, and policy
      enumeration.
- [x] Keep client certificates transport-only; native client-certificate
      authentication and certificate-to-principal mapping remain out of scope.
      If certificate identity is mapped into authorization later, define issuer
      trust, name constraints, revocation, renewal, and its interaction with
      bearer credentials before enabling the mapping.
- [x] Add integration tests for TLS versions and cipher policy, invalid and
      expired certificates, plaintext/downgrade attempts, reload, IPv4/IPv6
      exposure, resource limits, hostile origins, proxy spoofing, delegation
      injection, dual-identity audit attribution, revocation, and secret
      redaction.

## Hardware plugins

### Alienware

- [ ] Hardware-validate the externally corroborated AW-ELC profiles for the
      Dell G5 SE 5505, G7 15 7500, G15 5511, and G15 5530.
- [ ] Collect safe identity reports for unknown `187c:0550` and `187c:0551`
      platforms before adding profiles; do not infer topology from the USB ID.
- [ ] Research older AlienFX controller families and desktop hardware without
      treating them as AW-ELC-compatible by default.

### Govee

- [ ] Complete the plugin implementation and validate discovery, topology,
      control, readback, reconnect, and packaging against real devices.
- [ ] Broaden device support, including different types of devices.
- [ ] BLE support.

### Philips Hue

Detailed work lives in the [plugin plan](plans/philips-hue-plugin-plan.md) and
[setup API plan](plans/plugin-setup-api-plan.md).

- [ ] Run container-specific and macOS packaging checks.
- [x] Validate mDNS discovery, TLS identity, push-link setup, LCA007 mutations
      and readback, and LCA007/LCD006 enumeration against a physical BSB002
      bridge.
- [ ] Validate explicit endpoints, credential rejection, additional supported
      light types, topology and DHCP changes, Hue-app and wall-control
      coexistence, and request pacing against physical hardware.

Later Hue phases (credential removal or rotation, structured multi-bridge
configuration, rooms/zones/scenes, events, gradients, and Entertainment API
streaming) remain deferred until the initial hardware-validated milestone is
complete and a concrete need is established.

Richer generic setup interactions (progress, structured or sensitive input, and
external actions) are deferred until a concrete plugin needs them.

### Keychron ([detailed plan](plans/keychron-plugin-plan.md))

Blocked on access to a reference keyboard.

- [ ] Record exact hardware and Raw HID identity, validate firmware LED mapping,
      and add an exact profile, stable topology, and narrowly scoped udev rule.
- [ ] Validate volatile per-key HSV get/set and power-cycle behaviour, then
      implement static colour, brightness, off, batching, and truthful readback.
- [ ] Expose `SaveCurrent` only after a complete persistable session shadow is
      established and hardware EEPROM semantics are verified.
- [ ] Investigate firmware effects, mixed regions, and a measured streaming rate.
- [ ] Add other models only with exact identity, topology, feature, and protocol
      evidence.

### Razer ([detailed plan](plans/razer-plugin-plan.md))

- [ ] Finish the BlackWidow V4 Pro physical map, including wrist-rest ordering
      and remaining auxiliary-zone boundaries.
- [ ] Complete live effect, brightness/readback, persistence, streaming-rate,
      unplug/reconnect, restart, suspend/resume, hotplug, and input-coexistence
      validation; promote the profile only when the full checklist passes.
- [ ] Continue importing safely representable lighting profiles in protocol
      families, with exact selectors, fixtures, honest validation status, and a
      recorded blocker for each omitted family.
- [ ] Add the remaining transport families only when bounded: single-zone and
      classic matrix, mice/accessories, wireless/receiver routing, ARGB, and legacy
      reports.
- [ ] Finish narrow device permissions, package integration, sanitized user
      reporting instructions, and the upstream refresh/drift workflow.
- [ ] Define migration/alias handling before replacing published coordinate-only
      topology with named physical elements.

### Existing plugins and code-derived gaps

- [ ] Validate WLED and Govee on real devices, including on macOS.
- [ ] Validate owned LIFX Z/Beam hardware.
- [ ] Implement and hardware-test Alienware keyboard morph through the custom
      animation path, or remove the advertised operation if it cannot be truthful.
- [x] Implement and hardware-test Alienware keyboard firmware `SaveCurrent`, or
      keep persistence explicitly unsupported in its capabilities.
- [ ] Ensure future USB/HID plugins re-enumerate live during topology probes
      instead of caching construction-time discovery.

## Platforms and packaging

### Windows ([detailed plan](plans/windows-service-plan.md))

- [ ] Exercise the service device-arrival-to-rescan path with supported physical
      hardware and confirm one debounced `RescanReason::DeviceChange` per event.
- [ ] Build the WiX v4 MSI around the existing service installer, including
      Program Files payload staging, install/start options, safe major upgrades,
      and non-purging uninstall behaviour.
- [ ] Add Windows MSI build, install, upgrade, and removal coverage to CI.

ETW/TraceLogging, a virtual service account, and a Windows reload signal remain
deferred investigations, not release tasks.

### macOS ([detailed plan](plans/macos-portability-plan.md))

- [x] Add automated Intel and Apple Silicon CI coverage equivalent to the
      existing manual contribution workflow.
- [ ] Validate launchd `RunAtLoad` across a real reboot.
- [ ] Exercise a real sleep/wake cycle and physical hotplug event end to end.
- [ ] Validate WLED and Govee against real devices on macOS.
- [ ] Design and ship signed/notarized package-manager distribution.

## Product and ecosystem

- [ ] Investigate Logitech compatibility
- [ ] Investigate OpenRGB compatibility
- [ ] Investigate macOS keyboard-backlight support
- [ ] Explore Home Assistant integration

Matter and HomeKit are future directions rather than current commitments.

## UI notes

GUI and TUI development is continuing out of tree.

The TUI is roughly on-par with `luminatectl` where it makes sense.

The Qt GUI is experimental and in-progress, but basic functionality works.

## Audit notes

The source audit performed when this file was created found no explicit
`TODO`, `FIXME`, `todo!`, or `unimplemented!` markers under `crates/`,
`plugins/`, `packaging/`, or `scripts/`. The Alienware items above come from
explicit runtime "not implemented yet" paths. Unsupported behaviour that is
documented as a deliberate boundary is not automatically a task.
