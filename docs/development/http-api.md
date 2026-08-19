<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# HTTP API

`luminate-http` is an optional network companion that maps authenticated REST
and WebSocket requests onto the unified daemon client. It is intended to
support remote clients over a LAN; it is not a loopback-only interface. Its API
remains pre-release and compatibility-sensitive. The OpenAPI document is
available without authentication at `/api/v0/openapi.json`.

## Authentication and authorization

REST requests present a daemon bearer secret as URL-safe base64 in
`Authorization: Bearer …`. The companion treats it as opaque and opens a new
daemon `Client` using bearer authentication for the request. `luminated`
verifies the credential, establishes the canonical subject and lease, and
performs every authorization decision. The companion has no token, policy,
ownership, quota, or audit store of its own.

`GET /api/v0/me` reports sanitized daemon session metadata: token ID,
authority, subject, verified groups, and any delegated front-end actor. It
never returns the bearer secret. Token administration endpoints call the
daemon's grouped authentication-administration API; newly created or rotated
secrets appear only in that successful response.

The HTTP API deliberately does not expose general attestation administration.
Daemon attestations are bound to the connecting kernel actor, which would be
the `luminate-http` process rather than the remote HTTP caller. Delegation uses
attestations only as an internal, one-use bridge and never returns their
secrets across the HTTP boundary.

Trusted delegation is available only in `trusted-proxy` authentication mode.
The peer address must match `--trusted-proxy`, and its bearer must match the
dedicated secret in `--trusted-proxy-credential-file`. The proxy must strip any
client-supplied `Luminate-Delegation` header before supplying its own bounded
claim. The companion authenticates the front-end bearer with the daemon,
obtains an actor-bound one-use attestation, and reconnects as the delegated
principal. The daemon preserves both identities and intersects the delegated
principal's policy with the immutable front-end actor ceiling. Direct and
insecure-development modes reject delegation headers.

The header is a bounded, unpadded-base64url JSON claim with exactly
`authority`, `subject`, `verified_groups`, and `expires_at` fields. Duplicate,
unknown, empty, oversized, malformed, or expired claims are rejected. The
attestation expires at the earliest of the claim, bearer lease, and the
companion's 30-second hard lifetime. Delegation failure never falls back to the
bearer identity.

Authentication failures return `401`; authenticated policy denials return
`403`. Expired or revoked credentials close daemon sessions, so a long-running
operation cannot retain access merely because its HTTP request began earlier.

Bootstrap the first remote credential through a locally authenticated daemon
client with `manage-authentication` authority. Create a separate token ID and
the narrowest useful policy binding for each person or device; do not share one
household token or treat the companion's service identity as the remote user.
Deploy only the returned secret to that client, using private storage, and set
an expiry when the client can renew safely. The bearer secret is replayable, so
TLS and storage protection remain necessary even for a narrowly authorized
token.

Rotate without an outage by creating or rotating the credential, deploying the
replacement secret, confirming that the client authenticates with its expected
token ID and principal, and only then revoking the old credential. Revoke a
lost or retired credential immediately. Recovery is deliberately local: use a
peer-authenticated daemon administration client or another separately held
administrative credential to create a replacement. The HTTP companion has no
recovery secret and cannot restore a revoked token.

## Health and exposure

`GET /health/live` reports that the companion process is running.
`GET /health/ready` also checks an ordinary peer-authenticated daemon
connection. Neither endpoint accepts a bearer credential.

`GET /api/v0/ping` is the authenticated counterpart: it opens a session with
the caller's bearer credential and sends the daemon's ordinary ping operation.
`GET /api/v0/server` reports the `luminate-http`, `libluminate`, and daemon
versions, the daemon implementation name, and the protocol ABI version.

Device responses and event baselines use the daemon's complete serialized
`Device` representation. Their ordered `physical_tags` field describes the
fixture or peripheral as a whole; nested surface `physical_tags` describe only
that addressable region or mapping; and nested element `physical_tags`
describe one individually addressable physical object. Tags are not inherited
between scopes. They are open presentation metadata, so clients must preserve
unknown values and ignore them safely. The [physical-tag
reference](physical-tags.md) documents the standard vocabulary and
plugin-defined extensions. Protocol ABI 29 introduced the element-level field;
no separate runtime HTTP-only topology model or version is involved. The
OpenAPI document uses HTTP-owned schema DTOs to describe that canonical JSON
without coupling the core and daemon protocol crates to the OpenAPI generator.
Open capability, policy, and configuration subdocuments are marked as
extensible objects so generated clients preserve additions they do not yet
understand.

The packaged service defaults to loopback so installation does not
unexpectedly expose a network service. A non-loopback listener requires a TLS
certificate and private key unless `--authentication-mode
insecure-development` is explicitly selected. That escape hatch logs a warning
and is never a safe bearer-token deployment.

Direct LAN operation uses `--tls-certificate` and `--tls-private-key`.
Certificates may be issued by a public CA, a private LAN CA, or be self-signed;
clients are responsible for trusting the chosen certificate and verifying the
name or IP address in it. The leaf certificate must be currently valid and
contain at least one DNS or IP subject alternative name. `luminate-http` warns
when the leaf has 30 days or less remaining. It rejects malformed, expired, or
not-yet-valid certificates at startup and reload.

The native listener accepts TLS 1.2 and TLS 1.3 and rejects older protocol
versions and plaintext. Cipher suites are the effective defaults of the
configured Rustls AWS-LC provider; they are intentionally not duplicated as a
fixed list here because Rustls, the provider, and distribution policy own that
list. Operators with a suite allowlist should inspect the deployed build and
test it with their TLS scanner rather than relying on a stale documentation
promise.

Obtain the certificate from a public CA, a private LAN CA trusted by every
client, or a self-signed workflow which pins that certificate in every client.
Keep the CA signing key separate from this host. The private key must be a
regular PEM file and, on Unix, deny all group and other access.

Renew before the 30-day warning threshold. Write the complete replacement
certificate chain and matching private key to temporary files in the same
filesystem, set the key's private permissions, and rename each file over its
configured path. Replace both promptly, then send `SIGHUP` on Unix or wait for
the 30-second reload interval. A successful signal reload is logged. A failed
reload leaves the last valid pair active, so restore the previous files and
reload again to roll back. Because the two path replacements cannot be one
filesystem operation, a periodic reload may briefly observe a mismatched pair;
that attempt is safely rejected and the next interval retries it.

If a key or certificate is compromised, issue a replacement with a new private
key, deploy it as above, and remove trust in the old self-signed certificate or
revoke it through the issuing CA. The companion cannot publish or enforce CA
revocation for its own server certificate. Confirm that clients trust the new
chain and verify its configured DNS name or IP address before retiring the old
trust material.

The listener defaults to 128 simultaneous connections, 1 MiB request bodies,
32 KiB of request headers, and 64 header fields. A client has five seconds to
finish its headers and 30 seconds to finish a request. An otherwise idle HTTP
keep-alive connection closes after 30 seconds. TLS handshakes have a ten-second
limit. The corresponding options are `--maximum-connections`,
`--maximum-request-bytes`, `--maximum-header-bytes`, `--maximum-headers`,
`--request-header-timeout-seconds`, `--request-timeout-seconds`,
`--keep-alive-idle-seconds`, and `--tls-handshake-timeout-seconds`.

Rate limiting uses separate token buckets. Defaults are:

| Dimension | Per minute | Initial burst | Options |
| --- | ---: | ---: | --- |
| Every request from a network peer, before authentication | 120 | 40 | `--pre-auth-requests-per-minute`, `--pre-auth-request-burst` |
| Health and OpenAPI requests from a peer | 30 | 10 | `--public-requests-per-minute`, `--public-request-burst` |
| Failed authentication from a peer | 20 | 5 | `--failed-auth-per-minute`, `--failed-auth-burst` |
| Failed authentication for one credential digest | 10 | 3 | `--failed-credentials-per-minute`, `--failed-credential-burst` |
| Authenticated requests from a peer and canonical credential ID (plus delegated subject, when present) | 60 | 20 | `--requests-per-minute`, `--request-burst` |
| WebSocket upgrades from a peer | 30 | 10 | `--websocket-upgrades-per-minute`, `--websocket-upgrade-burst` |

Each dimension has a separate store with deterministic oldest-entry eviction;
`--maximum-rate-limit-entries` sets the per-store capacity and defaults to
4096. Before authentication, a SHA-256 credential digest may be used only as
an in-memory limiter key. It is never logged or persisted. Authentication
failures remain the same `401` response when that credential bucket is
exhausted, so throttling does not become a credential oracle.

Rate-limit network identity comes from the TCP peer address, not a
client-controlled forwarding header. `Forwarded`, `X-Forwarded-*`,
`X-Real-IP`, and related forwarding headers are removed before routing. They
cannot select a peer or identity, including in trusted-proxy mode.

Browser access is disabled by default. Configure each permitted browser
application with a repeatable exact `--allowed-origin
https://host.example[:port]` option. Origins are matched by scheme, host, and
port; wildcards, `null`, user information, and origins containing paths,
queries, or fragments are rejected. Plain HTTP origins are accepted only for a
loopback host in `insecure-development` mode. Requests without `Origin` remain
available to non-browser clients.

Allowed browser applications receive narrowly scoped CORS responses for the
API's methods and the `Authorization` and `Content-Type` request headers. The
companion does not allow cookie credentials. Keep bearer tokens in memory and
out of cookies, URLs, local storage, logs, and crash reports. Obtain a one-use
WebSocket ticket immediately before opening a socket; the same origin policy
is checked before that ticket is consumed.

A TLS reverse proxy may connect to a loopback listener in `trusted-proxy` mode.
Configure every permitted proxy address and a private file containing its
unpadded-base64url daemon bearer secret. The credential file follows the same
private-file ownership and access checks as the TLS key. Secure non-loopback
proxy-to-companion hops with the companion's native TLS. Never run
`luminate-http` as root or as the daemon account: doing so would inherit
intrinsic break-glass authority.

The proxy must replace rather than append `Luminate-Delegation`. Native HTTP
forwarding headers are still stripped and are never an identity channel.

For example, a direct self-signed LAN deployment can use:

```sh
luminate-http --listen 192.0.2.10:8443 \
  --tls-certificate /etc/luminate/http.crt \
  --tls-private-key /etc/luminate/http.key
```

The packaged systemd unit remains loopback-only. For LAN service, create a
drop-in which clears and replaces `ExecStart` with the required TLS and other
options, then run `systemctl daemon-reload` and restart the service. OpenRC
deployments can set `luminate_http_listen` and `luminate_http_options` in the
service conf.d file. Treat proxy credentials and private-key paths in service
configuration as sensitive operational data even though the files, rather
than their contents, are named on the command line.

## WebSockets

Bearer and delegation headers are not carried into a WebSocket session.
An authenticated REST request first creates a short-lived, one-use companion
ticket, then uses it in the events or frames upgrade URL. The companion removes
the ticket-bearing query before request tracing. Tickets are endpoint-bound,
expire quickly, and cannot be reused.

At most 64 WebSocket sessions are active by default. Upgrades reject messages
and frames larger than 1 MiB, event output uses a 16-message bounded queue, and
a frame stream must send its start message within five seconds. Event and frame
sockets send pings and close after a missed 60-second liveness interval. These
limits are configured with `--maximum-websocket-sessions`,
`--maximum-websocket-message-bytes`, `--maximum-websocket-queue`,
`--websocket-start-timeout-seconds`, and `--websocket-idle-seconds`.

The upgrade operations include an `x-luminate-websocket-messages` OpenAPI
extension. It links the events server messages and frames client/server
messages to component schemas. The frame operation also identifies the
companion binary `LFRM` protocol documented below.

The event socket owns an authenticated daemon client and receives daemon event
tickets through that primary session. It therefore inherits the same subject,
authentication lease, voluntary scope, and policy context. Frame sockets bind
to one authorized target and negotiated stream generation. Authentication
expiry, revocation, ticket failure, or daemon disconnect closes the socket.

### Event subscriptions

An events ticket can elect its initial baseline, event kinds, and devices or
collections. Omitting `subscription` preserves the default behaviour: send the
complete observable device baseline and then every event.

```json
{
  "purpose": "events",
  "subscription": {
    "include_baseline": true,
    "kinds": ["topology_changed", "state_changed"],
    "selectors": [
      { "kind": "device", "id": "desk-lamp" },
      { "kind": "collection", "id": "work-room" }
    ]
  }
}
```

An empty `kinds` list selects every event kind. An empty `selectors` list
selects every device. Collection membership is expanded once when the socket
opens. Device selectors filter the baseline and target-bearing events;
configuration, scene, and transition events remain global and are filtered
only by `kinds`. `resync_required` bypasses both filters because it discloses no
resource information and means the client must fetch every selected baseline
again before trusting later incremental events.

Filtering reduces data sent to the HTTP client. It is not an authorization
boundary: the daemon still authorizes the subscription and `luminate-http`
receives the caller's observable event stream before applying the filter.

### Frame uploads

Create a frame ticket with `{"purpose":"frames"}`, upgrade
`/api/v0/ws/frames?ticket=…`, and send this text message first:

```json
{
  "type": "start",
  "protocol": 0,
  "target": { "Device": "desk-lamp" }
}
```

The server replies with `{"type":"started","generation":N}`. Subsequent
text messages carry the same `FramePayload` representation as `libluminate`:

```json
{
  "type": "frame",
  "sequence": 1,
  "payload": { "Full": [{ "Rgb": { "r": 255, "g": 80, "b": 16 } }] },
  "commit": true
}
```

Each accepted upload receives an `ack` containing its sequence number and the
daemon's `dropped` flag. Closing the socket ends the negotiated stream.

For dense RGB8 frames, a binary message avoids JSON overhead. Its 24-byte
big-endian header is: ASCII magic `LFRM`; header version `0`; pixel format `1`
(RGB8); 16-bit flags (`bit 0 = commit`); 64-bit sequence; 32-bit pixel count;
and four reserved zero bytes. Exactly `pixel_count * 3` interleaved RGB bytes
follow. Unknown flags, formats, non-zero reserved bytes, and length mismatches
close the stream with an error. Shared-memory streaming remains a same-host
`libluminate` optimization and is not exposed through HTTP.

## Operations and errors

Named appearance slots use the `appearance-slots` control operation. It takes
one concrete surface and an ordered `values` array; it deliberately has no
selector fan-out form:

```json
{
  "operation": "appearance-slots",
  "target": { "Surface": { "device": "alienware-aw-elc", "surface": "power-button" } },
  "values": [
    { "slot": "ac", "effect": { "Static": { "colour": { "Additive": [
      { "channel": "Red", "value": 0 },
      { "channel": "Green", "value": 128 },
      { "channel": "Blue", "value": 255 }
    ] } } } }
  ]
}
```

Topology, state, and scene resources serialize the canonical Rust model, so
slot IDs, descriptors, update policy, known values, and the `complete` marker
remain explicit in JSON.

REST operations use the same daemon semantics as the Rust client, including
selector resolution, partial collection outcomes, scene ownership,
transitions, topology-generation conflicts, and admission limits. HTTP does
not reinterpret a denial or retry an operation under another identity.

Errors use problem responses with a stable type, HTTP status, safe detail,
request ID when available, retry guidance when applicable, and applied-target
metadata for partial operations. Credentials, hidden resources, daemon-private
paths, provider continuations, and sensitive configuration values must not
appear in responses or tracing.
