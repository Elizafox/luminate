<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Windows Event Log events

`luminated` writes a small set of operational events to the Windows
`Application` log under the `luminated` source. Detailed tracing belongs in the
rotating service log instead; the Event Log vocabulary is intentionally
limited to events an administrator may need to monitor or act upon.

The source and numeric Event ID together form a compatibility-sensitive
interface. New IDs may be added, but an assigned ID must not be reused for a
different event. Retired IDs remain reserved.

## Allocated ranges

| Range | Meaning                                             |
| ----- | --------------------------------------------------- |
| 1xx   | Service lifecycle (start, ready, stop, preshutdown) |
| 2xx   | Configuration and persisted state                   |
| 3xx   | Transport and listeners                             |
| 4xx   | Plugin host and supervision                         |
| 5xx   | Reserved for power and device events                |

## Event catalogue

| ID  | Event                         | Meaning |
| --- | ----------------------------- | ------- |
| 100 | Service starting              | The Windows service began initialization. |
| 101 | Service ready                 | The daemon is accepting client connections. |
| 102 | Service stop requested        | SCM requested an ordinary service stop. |
| 103 | Service preshutdown requested | SCM requested a stop during system preshutdown. |
| 104 | Service stopped               | The Windows service stopped. |
| 105 | Service file logging degraded | The rotating file sink could not be initialized. |
| 200 | Configuration load failed     | The daemon could not load its configuration. |
| 201 | Persisted state load failed   | The daemon could not restore persisted state. |
| 202 | Persisted state save failed   | The daemon could not save persisted state. |
| 300 | Listener bind failed          | A client transport listener could not be bound. |
| 400 | Plugin host crashed           | A supervised plugin host terminated unexpectedly. |

The canonical definitions live in `crates/luminated/src/operator_event.rs`.
The Event Log tracing layer must accept only events emitted through that typed
vocabulary, rather than treating ordinary tracing events as operator events.
Operator messages escape C0, DEL, and C1 controls before reaching either the
Event Log or the rotating file sink, preventing forged records and terminal
sequences when administrators inspect exported logs.
