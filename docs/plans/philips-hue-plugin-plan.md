<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Philips Hue plugin plan

## Status

In progress. Configuration, bounded mDNS discovery, the Hue-specific TLS trust
policy, bounded authenticated HTTP/1.1 transport, validated API v2 bridge,
device, and light enumeration, capability-accurate topology, validated light
mutations, bounded state readback, paced discovery, registry expiry, rescans,
topology notifications, packaging integration, configuration examples, and
user documentation are implemented. The full contribution workflow and native
Linux staged-install smoke test pass. Physical validation covers the core
discovery, setup, enumeration, mutation, and readback path described below.
Container-specific and macOS packaging verification and the remaining hardware
matrix remain outstanding.

This plan targets the local Philips Hue Bridge API v2. It supports both an
application key created outside Luminate and the implemented push-link setup
workflow. API v1 is out of scope unless a concrete compatibility need appears.

The plugin is experimental. Its configuration, device IDs, topology, capability
mapping, and supported resource set may change as bridge and light hardware are
validated. Changes must still be coherent across code, tests, packaging, and
documentation; experimental status is not permission to weaken validation or
silently reinterpret state.

## Goal

Add local, authenticated control of lights connected to a Philips Hue Bridge
without using a Hue cloud account or remote API. The first version will discover
or connect to one configured bridge, authenticate over HTTPS, publish each
supported API v2 `light` resource as a Luminate device, apply ordinary lighting
state, and read that state back.

The initial implementation should be useful for conventional white, colour-
temperature, and colour lights while remaining honest about resource
capabilities and bridge reachability. It must not guess that every Hue light has
RGB or colour-temperature support, manufacture geometry for gradient products,
or interpret a successfully submitted bridge request as proof that an
unreachable Zigbee light changed state.

## Decisions

- Use the local Hue Bridge API v2 over HTTPS.
- Keep API v1 out of the implementation. If users later need it, design it as a
  distinct compatibility path rather than mixing v1 and v2 resource models.
- Accept a pre-created application key as sensitive plugin configuration, or
  create one through the push-link setup workflow. Never include it in
  diagnostics.
- Represent exactly one bridge per plugin instance initially. The current
  settings schema cannot truthfully associate an arbitrary number of bridge
  identities, endpoints, and sensitive keys without parallel arrays or encoded
  mini-languages. Multi-bridge support therefore waits for a suitable structured
  configuration model or independently configurable plugin instances.
- Use bridge-local discovery only. Do not contact `discovery.meethue.com` or any
  other cloud service.
- Recommend `Adopt` reconciliation because lights may also be changed by wall
  controls, the Hue app, automations, and other local clients.
- Treat individual API v2 `light` resources as the first public topology.
  Rooms, zones, grouped lights, scenes, and entertainment configurations remain
  separate future work rather than being flattened into duplicate devices.
- Do not use the REST API for frame streaming or rapid client-timed effects.
  Signify directs continuous fast updates to the Entertainment API.

## Sources and assumptions

The public Signify material available without an authenticated developer portal
establishes the main constraints used here:

- the local API should be accessed over HTTPS, and API v2 does not support
  HTTP: <https://developers.meethue.com/new-hue-api/>;
- mDNS and `discovery.meethue.com` are the recommended discovery mechanisms,
  while UPnP is deprecated: <https://developers.meethue.com/new-hue-api/> and
  <https://developers.meethue.com/develop/get-started-2/>;
- local access requires the client to be on the same network as the bridge:
  <https://developers.meethue.com/explore/why-develop-for-hue/>;
- push-link creates a randomly generated application credential after physical
  confirmation on the bridge: <https://developers.meethue.com/develop/get-started-2/>;
  and
- REST updates are not intended for continuous fast lighting updates:
  <https://developers.meethue.com/new-hue-api/>.

The complete v2 schema and HTTPS connection guidance require a Hue developer
login. The official API reference, application design guidance, discovery
guide, HTTPS certificate guidance, performance guidance, and colour conversion
guidance were reviewed from a locally saved copy on 8 August 2026. Verify every
endpoint, header name, resource field, colour conversion rule, rate
recommendation, and certificate-validation rule against those sources. Do not
promote examples from third-party clients into protocol guarantees without
corroboration.

Signify announced in June 2025 that new bridge firmware would remove HTTP
support and require TLS. That reinforces the v2-only decision and rules out a
plaintext bootstrap or fallback.

## Initial scope

The first experimental milestone includes:

- one configured v2 bridge identified by its canonical bridge ID;
- an application key supplied as a sensitive restart-required setting;
- an optional explicit bridge endpoint for routed, isolated, or test networks;
- bounded mDNS discovery when no explicit endpoint is configured;
- HTTPS server authentication following Signify's current bridge certificate
  guidance, with the configured bridge identity bound to the TLS peer;
- authenticated enumeration of API v2 `bridge`, `device`, and `light`
  resources needed to establish identity, metadata, ownership, and capability;
- one Luminate device for each supported `light` resource;
- exact per-resource capabilities derived from the fields and schemas reported
  by the bridge;
- on/off, brightness, CIE xy colour, and colour-temperature writes where the
  corresponding resource advertises them;
- bounded state readback for those same facets, including reachability and
  validity indicators where the API provides them;
- topology refresh when lights are added, removed, renamed, or change reported
  capabilities; and
- deterministic protocol, TLS-policy, discovery, topology, conversion,
  mutation, readback, and error-classification tests.

The initial implementation deferred push-link pairing and application-key
persistence. They are now implemented through the generic setup API. The
following remain deferred:

- multiple bridges in one plugin instance;
- cloud discovery, remote access, OAuth, and Hue accounts;
- API v1 and first-generation round-bridge support;
- rooms, zones, `grouped_light` resources, and Luminate collections;
- bridge scenes, dynamic scenes, smart scenes, and firmware effects;
- gradient segments and guessed strip or luminaire geometry;
- Entertainment API streaming and DTLS;
- sensors, switches, cameras, security features, and MotionAware resources;
- configuration, deletion, or firmware management of Hue resources; and
- automatic registration or deletion of bridge application credentials.

## Configuration

The proposed settings are:

```toml
[[plugins]]
name = "luminate-plugin-philips-hue"
reconciliation = "Adopt"

[plugins.config]
bridge_id = "001788fffe012345"
application_key = "replace-with-the-bridge-created-key"
mdns = true
# endpoint = "192.0.2.20:443"
```

`bridge_id` and `application_key` are required strings. `application_key` is
sensitive. `mdns` defaults to `true`. `endpoint` is optional; when present it
is authoritative for routing, but the TLS and authenticated bridge identity
must still match `bridge_id`. An explicit endpoint must never turn off peer
authentication.

Configuration parsing will deny unknown fields and validate all values before
starting discovery. Normalize the bridge ID into one canonical representation
only after validating its official syntax. Do not log the application key,
place it in an environment variable, include it in an error, or retain it in a
debug-printable structure. The implementation should keep the key in a narrow
redacted or zeroizing type if an existing workspace abstraction is suitable;
otherwise the dependency and secret-lifetime design needs review before code is
written.

The first implementation will not accept an application key without a bridge
ID. That pairing is the stable security boundary even if DHCP changes the
bridge address. Discovery results for other bridges are ignored rather than
probed with the configured credential.

## Architecture

`plugins/luminate-plugin-philips-hue` will be an independent `cdylib`/`rlib`
hardware plugin with the following internal layers:

1. `configuration` owns the typed single-bridge settings, secret redaction,
   endpoint parsing, and semantic validation.
2. `mdns` sends bounded Hue service queries and parses PTR, SRV, TXT, A, and
   AAAA records without trusting unrelated response data. It returns candidate
   endpoints and advertised bridge identities, not authenticated devices.
3. `tls` constructs the bridge TLS client and enforces the current official Hue
   certificate and identity rules. Tests exercise trusted, mismatched, expired,
   malformed, and otherwise rejected peers. There will be no general
   “accept-invalid-certificates” mode.
4. `http` is a bounded HTTPS/HTTP 1.1 transport with request deadlines, response
   size limits, strict status parsing, content-type checks, and the Hue
   application-key header. It must handle the response framing actually allowed
   by the official bridge guidance without unbounded buffering.
5. `api` owns v2 paths, request and response DTOs, API-level error envelopes,
   resource references, capability fields, and mutation builders. Unknown JSON
   fields remain forward-compatible; missing required identity or capability
   fields produce useful errors.
6. `colour` converts between Luminate colour values and Hue CIE xy plus
   brightness. It applies a light's reported colour gamut when the official
   algorithm requires it, with deterministic boundary tests. Readback fidelity
   will reflect the lossy conversion rather than claim an exact RGB round trip.
7. The plugin runtime owns the dynamic registry, discovery pacing, topology
   fingerprints, request throttling, and read/apply integration with the plugin
   API. Network I/O remains behind injectable traits for hermetic tests.

Use the existing `DynamicDeviceRegistry`, `DiscoveryPacer`, request deadline,
and plugin configuration facilities where they fit. The WLED implementation is
a useful source for bounded HTTP and mDNS parsing, but Hue-specific HTTPS,
authentication, discovery records, response semantics, and resource modelling
must remain distinct. Shared networking code should only be extracted after a
second implementation demonstrates a genuinely identical abstraction.

### Dependency checkpoint

The user approved `rustls`, `rustls-pki-types`, `zeroize`, and `x509-parser`.
Rustls performs chain, signature, validity-period, intermediate, and handshake
signature validation against Signify's two published Hue roots. `x509-parser`
is narrowly used to require the leaf certificate's single Common Name to match
the configured canonical bridge ID, as required by Hue's certificate guidance.
The transport does not provide an insecure mode or HTTP fallback. No general
HTTP client was added. `cargo audit` reported no known vulnerabilities after
the dependency changes.

## Discovery and identity

Discovery is an untrusted hint. The implementation must confirm the configured
bridge identity through the TLS policy and an authenticated v2 bridge resource
before publishing any lights.

The discovery design must establish from official documentation:

- the exact DNS-SD service name and required TXT keys;
- whether bridge ID matching is case-sensitive on the wire;
- how IPv4 and IPv6 endpoints are associated with SRV targets;
- whether the advertised service port is always 443; and
- the supported relationship between bridge ID, certificate identity, and the
  authenticated v2 bridge resource.

Apply bounded packet counts, record counts, compression-pointer traversal,
response sizes, discovery windows, candidates per source, and total registry
size. As with WLED, an unauthenticated response must not redirect a probe to an
unrelated address through DNS records supplied by another host. Reject
ambiguous identity rather than choosing the first bridge.

An explicit endpoint may be a numeric IPv4 or IPv6 address or a resolvable host
and port, subject to the same bounded-resolution rules used by other network
plugins. Device IDs should be derived from the canonical bridge ID and stable
v2 resource UUID, for example under a plugin-owned namespace. Display names,
IP addresses, discovery order, and legacy v1 IDs are not identities.

## Topology and capability mapping

Each supported v2 `light` resource becomes one Luminate bulb-category device
with one whole-device light-emitting surface. The descriptor name comes from
the owning Hue device metadata when that relationship is present and
unambiguous. If the bridge cannot establish a safe owner or stable identity,
omit the resource with a diagnostic rather than inventing one.

Capabilities are structural and resource-specific:

- `on` maps to emission control;
- `dimming` maps to brightness using the documented percentage range and
  minimum dim level;
- `color` maps to CIE xy colour constrained by the reported gamut;
- `color_temperature` maps from mirek bounds to Luminate Kelvin bounds with
  checked, saturating-at-the-declared-boundary conversion; and
- connectivity or service status informs availability but is not exposed as a
  lighting capability.

The implementation must determine from existing Luminate conventions whether
Hue's logical `on` state truthfully maps only to `EmissionState` or also to a
device physical-power facet. The likely answer is emission only: a bridge-
connected lamp remains electrically powered while logically off. This must be
confirmed against current core semantics before descriptors are finalized.

Do not advertise RGB for white-only or colour-temperature-only lights. Do not
advertise colour temperature when the bridge omits it. Preserve unknown or
temporarily invalid state such as `mirek_valid = false`; do not replace it with
a default temperature. Gradient light resources remain a single whole-light
device until their segment ordering and geometry can be represented truthfully.

Topology fingerprints include every field that changes the public descriptor:
resource UUID, owning device identity and name, product metadata used in the
descriptor, supported facets, gamut, temperature bounds, and any availability
classification that structurally changes exposure. Pure state changes do not
churn topology.

## Requests, state, and reconciliation

All writes target the v2 light resource UUID with minimal JSON payloads. A
single Luminate update should become one bridge mutation where the v2 schema
allows its facets to be combined. Validate the complete update before network
I/O so an unsupported facet cannot cause a partial write.

Hue API success and error arrays must be inspected even when HTTP status is
successful. Classify at least invalid arguments, unsupported operations,
authentication rejection, unavailable or unreachable resources, rate limits,
timeouts, TLS failures, malformed responses, and internal registry failures
into appropriate `PluginError` or `PluginReadError` forms. Preserve a bounded
retry delay when the bridge reports one. Do not automatically retry ambiguous
mutations unless the API contract makes replay safe.

The runtime should rate-limit and coalesce only where doing so preserves
Luminate request semantics. Use current official guidance for limits; the
public Hue support page's historical guidance of roughly ten individual-light
commands per second is a conservative research input, not a substitute for the
v2 application design guidance.

Readback initially uses bounded authenticated GET requests. If one resource
snapshot can cover all requested facets, fetch it once and decode the requested
observations. Mark fidelity individually:

- logical on/off and brightness can be exact when valid and reported;
- colour-temperature can be exact after a reversible in-range mirek/Kelvin
  conversion only to the precision the protocol represents;
- RGB reconstructed from xy and brightness is best-effort; and
- a bridge-cached target state must not be described as observed physical light
  output when the light is unreachable.

An event-stream phase may later replace or supplement polling. It must include
reconnect, bounded parsing, snapshot resynchronization after gaps, and explicit
ordering semantics before event data is trusted for reconciliation.

## Security and privacy

- Require HTTPS and authenticate the intended bridge. Never provide an
  insecure certificate bypass, including for explicit IP endpoints.
- Treat mDNS, DNS, DHCP, IP addresses, resource names, product metadata, and all
  JSON returned by the bridge as untrusted input.
- Send the application key only to an endpoint already bound to the configured
  bridge identity by the TLS policy.
- Redact credentials from `Debug`, tracing fields, errors, tests, fixtures,
  management output, and documentation examples.
- Bound all network reads, JSON collections, strings retained in topology,
  discovery candidates, redirects, retries, and background work.
- Do not follow HTTP redirects. A redirect could disclose the application key
  to a different host.
- Keep all operation local to the configured LAN endpoint. No telemetry, cloud
  discovery, remote API, or account integration is in scope.
- Document that bridge resource names and topology can reveal household and
  room information to authorized Luminate clients.
- Avoid exposing cameras, security devices, presence history, or sensor data.
  Those resources are outside both the lighting goal and the initial privacy
  boundary.

## Push-link pairing phase

Pairing requires more than adding `POST /api`. It changes Luminate from a
consumer of administrator-supplied secrets into a credential provisioner and
therefore needs a reviewed end-to-end workflow.

The implemented workflow covers:

- [x] An administrator-authorized management operation that selects an
      authenticated bridge candidate and clearly asks the user to press its
      physical link button;
- [x] A bounded pairing window, cancellation, progress, and actionable
      `link button not pressed` reporting;
- [x] The official `devicetype` and application identity naming policy.
- [x] One-time receipt of the generated application key without logging or
      returning it through ordinary readable settings;
- [x] Durable storage through Luminate's managed sensitive-setting path, with
      atomic failure handling so a created bridge credential is not silently lost;
- [x] Bridge identity verification before and after credential creation.
- [x] Defined behaviour when configuration is locked, persistence fails, the
      bridge is already paired, or the credential is later revoked.
- [x] Define removal and rotation as local credential-lifecycle operations.
      Removal stops the plugin and securely discards Luminate's stored key even
      when the bridge is unavailable. Rotation requires push-link, verifies the
      replacement against the authenticated bridge, atomically persists it, and
      retains the old local key if creation, verification, or persistence fails.
      After a successful replacement, Luminate discards its old local key.
      Neither operation enumerates, identifies, or deletes bridge whitelist
      entries. Bridge-side revocation remains an explicit user action through
      Hue's application-management interface, and completion must not claim
      that local removal or rotation revoked the old bridge authorization.

Pairing uses the daemon/plugin setup interface rather than a startup loop,
environment-variable helper, implicit probe side effect, or unauthenticated
background action.

## Multi-bridge phase

Multiple bridges require a configuration representation that keeps each
canonical bridge ID, sensitive key, endpoint override, discovery policy, and
future pairing lifecycle in one validated object. Do not use positional
parallel arrays or strings containing ad hoc serialized records.

Before implementing this phase, decide whether the daemon will support multiple
instances of one plugin, extend setting kinds with arrays of tables, or provide
a dedicated credential/resource configuration model. Review the persistence and
management compatibility impact of that decision. Device IDs already include
the bridge ID so adding a second bridge will not collide with the initial
topology.

## Packaging and documentation

Implementation will require focused updates to:

- the root workspace membership and lockfile;
- Linux and macOS plugin build/install toggles following existing hardware
  plugins;
- the root README plugin list;
- the packaged configuration template;
- a plugin README with setup, key creation, security, experimental status,
  supported capabilities, and hardware-validation status; and
- relevant development configuration examples.

Use the descriptive plugin name `luminate-plugin-philips-hue` unless repository
packaging or naming review identifies a concrete problem. Plain-text
compatibility references may use “Philips Hue”; do not use vendor logos or imply
endorsement.

## Verification

### Hermetic tests

- configuration defaults, required values, unknown fields, bridge-ID
  normalization, endpoint parsing, and secret redaction;
- valid and hostile mDNS packets, compression cycles, truncated records,
  unrelated-source address injection, duplicate candidates, IPv4, and IPv6;
- TLS trust, hostname or bridge-ID binding, certificate expiry, wrong bridge,
  malformed chains, deadline expiry, and credential non-disclosure;
- HTTP status and framing, size limits, content types, redirects, rate limits,
  API-level success/error envelopes, and malformed JSON;
- v2 resource ownership, missing references, duplicate IDs, unsupported resource
  shapes, names, capability detection, and topology fingerprints;
- xy/RGB and mirek/Kelvin conversions at gamut vertices, edges, minimums,
  maximums, invalid values, and round-trip precision boundaries;
- mutation payloads for every supported facet and valid combination, plus proof
  that an invalid combined update performs no transport call;
- readback fidelity, invalid temperature state, unreachable lights, partial API
  errors, and request deadline propagation;
- registry expiry, rediscovery after DHCP changes, renamed and removed lights,
  authentication revocation, and deterministic request throttling; and
- exported descriptor, settings schema, probe, rescan, start, topology, apply,
  and read lifecycle behaviour.

Network tests should use local fake servers and injectable transports. Real Hue
credentials and household resource snapshots must not be committed as fixtures.
Sanitize captured schemas while retaining only the minimum structure needed for
regressions.

### Hardware validation

Use an explicitly configured ignored test or a documented isolated-daemon
procedure. Record bridge model, firmware/API version, and light product models
without recording application keys or private household names. Validate at
least:

- discovery and explicit endpoint operation;
- certificate validation through the supported IP and hostname paths;
- restart and DHCP-address changes;
- credential rejection and revocation;
- white-only, colour-temperature, colour, and an unreachable light if available;
- power, minimum and maximum brightness, gamut-edge colour, and temperature
  bounds;
- readback after writes from Luminate, the Hue app, and a wall control;
- topology updates after rename, add, and remove operations; and
- conservative sustained request behaviour without rapid-update flooding.

Gradient products, non-Signify Zigbee lights, IPv6, and Hue Bridge Pro must be
reported as untested until each is exercised. Software support inferred from
the common resource schema should be labelled accordingly.

#### 2026-08-12 physical validation

Validated a BSB002 bridge running firmware `1978074000` and API `1.78.0` on a
local IPv4 network. No credentials, household names, or resource snapshots are
recorded here.

- mDNS discovery returned the canonical bridge identity and IPv4 endpoint.
- Certificate-chain and canonical bridge-ID validation succeeded when routing
  to the discovered IP address.
- Push-link created an application key, verified it through API v2 enumeration,
  committed the sensitive setting atomically, and activated the plugin.
- Enumeration produced stable topology for six LCA007 colour lamps and two
  LCD006 colour downlights.
- One LCA007 lamp passed brightness `35` and `100`, RGB cyan, 2700 K colour
  temperature, off/on, and refreshed readback after every write. Brightness and
  emission read back as expected. RGB readback reflected documented gamut and
  CIE xy conversion loss; 2700 K read back as 2702 K after mirek quantization.
- The lamp was restored to emitting at brightness `100` and a best-effort pink
  appearance after the test.
- The bridge closed the pairing TLS connection without `close_notify`. The
  transport now retains received bytes on that EOF and relies on strict HTTP
  framing to reject a genuinely truncated response.

Explicit endpoints, hostname routing, restart and DHCP changes, credential
rejection and revocation, boundary values, other product families, topology
changes, Hue-app and wall-control coexistence, and sustained request pacing
remain untested.

### Contribution checks

During implementation, run directly affected tests early. Before completion,
run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Also run `cargo audit` for the dependency change and the relevant packaging
smoke tests. Report any checks that need real hardware, multicast networking,
elevated permissions, or an installed bridge certificate and could not be run.

## Implementation sequence

- [x] Obtain and record the current official v2, discovery, application design,
      and HTTPS certificate guidance. Resolve the TLS identity algorithm and exact
      mDNS service before code.
- [x] Review and approve the TLS dependency additions and secret representation.
- [x] Add the crate skeleton, settings schema, configuration validation, workspace
      membership, and descriptor tests.
- [x] Implement and fuzz-minded-test bounded mDNS parsing and explicit endpoint
      resolution.
- [x] Implement TLS and bounded HTTP transport with fake-server tests. Prove that
      wrong peers and redirects cannot receive the application key.
- [x] Implement typed v2 resource enumeration, ownership joins, topology, stable
      IDs, and capability mapping.
- [x] Implement validated writes, colour conversions, error classification, and
      throttling.
- [x] Implement state readback and the `Adopt` recommendation with fidelity tests.
- [x] Add discovery pacing, registry expiry, rescans, and topology notifications.
- [x] Add packaging, configuration examples, README documentation, and the root
      plugin index entry.
- [x] Run the full contribution workflow and native Linux staged-install check.
- [ ] Run the container-specific and macOS packaging checks.
- [ ] Complete the remaining physical hardware matrix and update support claims
      as explicit endpoints, failure cases, coexistence, topology changes, and
      request pacing are validated.
- [ ] Review remaining unknowns before considering rooms, scenes, events,
      multi-bridge configuration, credential removal or rotation, or
      Entertainment API work.

## Completion criteria

The first experimental milestone is complete when a configured second-
generation or newer Hue bridge can be discovered or reached explicitly; its
identity is authenticated over TLS before the key is sent; supported individual
lights appear with stable, capability-accurate topology; supported state can be
written and read without guessing; malformed, hostile, unreachable, and
unauthorized cases fail safely; documentation and packaging are synchronized;
and all available contribution checks pass.

Hardware-independent tests alone are not sufficient to broaden support claims.
The README and release notes must distinguish the physically validated bridge,
light, and operation paths from coverage inferred from the common resource
schema.
