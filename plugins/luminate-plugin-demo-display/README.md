<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Demo display plugin

`luminate-plugin-demo-display` is a virtual 16x16 addressable matrix display.
It exists to give the zero-copy shared-memory frame fast path
(`docs/development/architecture/shm-frame-streaming-phase1.md`) a real,
loadable `cdylib` to negotiate and stream against: this is the only bundled
plugin that implements `ShmFrameStreamingPlugin`.

The fictional hardware is the `Aurora Panel` from `Nebula Forge`.

It models:

- one device: `demo-led-display`
- no surfaces, elements, or groups: one addressable 16x16 (256-pixel) matrix
- `ShmFrameShape::Matrix { width: 16, height: 16 }`, `Rgb8` only
- both frame-streaming paths: the always-available CBOR path
  (`FrameStreamingPlugin`) and the opt-in shared-memory fast path
  (`ShmFrameStreamingPlugin`)
- no persistence or readback; it only comes alive under streaming

The plugin accepts every probe. It does not touch hardware.

## Why this plugin exists

Every other bundled plugin advertises `shm_frame: none`: none of today's
real hardware needs the shared-memory fast path yet, so retrofitting a
production plugin to prove it out would be artificial. While
`luminate-plugin-demo-bulb` proves out the ordinary CBOR frame-upload path end
to end, this plugin proves out fast-path negotiation (`shm_stream_begin`),
delivery (`shm_frame`), and teardown (`shm_stream_end`) against a real compiled
plugin host rather than an in-process fake.

## Capabilities and behaviour

`display_capabilities()` advertises RGB colour and a `frame_upload`
capability whose `shm` field carries:

- `pixel_formats: [Rgb8]`: the only format this plugin accepts over the
  fast path
- `shape: Matrix { width: 16, height: 16 }`: a two-dimensional
  shape, not the `Linear` shape every other capability in this codebase
  uses, so a producer computing a 2D effect can address rows and columns
  directly
- `max_rate_hz: None`: no rate ceiling of its own

`shm_stream_begin` rejects any format other than `Rgb8` and any pixel count
other than 256, and returns a `DisplayStream` that only tracks how many
samples it has applied. This is enough for tests and a manual smoke test log
line to confirm frames are actually arriving via shared memory rather than
silently falling back to the pipe.

## Update handling

Ordinary mutations and CBOR-path frame uploads both go through the same
target/shape validation as the shared-memory path, recording into the same kind
of in-memory shadow map used by `luminate-plugin-demo-bulb`. Its value proves
callback execution but is neither daemon state nor durable device storage.

## Generated ABI boundary

`luminate_export_plugin!` exports the ABI version and descriptor symbols the
daemon discovers in the `cdylib`, including the three shared-memory callback
symbols (`shm_frame: native`). The SDK adapter owns handle minting/reclaiming
and panic containment for those callbacks the same way it already does for
`apply`/`read_state`/`upload_frame`; this plugin implements only typed Rust
methods on `ShmFrameStreamingPlugin`.

See the
[`plugin-authoring guide`](../../docs/development/plugin-authoring.md) for the
author-facing contract and the
[`shared-memory fast-path design`](../../docs/development/architecture/shm-frame-streaming-phase1.md)
for its architecture.
