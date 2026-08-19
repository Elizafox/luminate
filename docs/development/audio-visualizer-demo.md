<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Audio visualizer demo

The `libluminate` audio visualizer plays a local audio file through the
computer and renders its spectrum on one or more Luminate targets. It requires
`ffmpeg` and `ffplay`.

From the repository root:

```sh
cargo run --release -p libluminate --example audio_visualizer -- \
  music.opus --device alienware-keyboard
```

Selections may be mixed and repeated:

```sh
cargo run --release -p libluminate --example audio_visualizer -- \
  music.flac \
  --surface alienware-keyboard/keyboard \
  --group case-lights/fans \
  --collection gaming-room
```

The accepted selectors are:

- `--device DEVICE`
- `--surface DEVICE/SURFACE`
- `--group DEVICE/GROUP`
- `--collection COLLECTION`

Set `LUMINATED_SOCKET_PATH` when the daemon does not use the client library's
compiled default path.

The example recursively resolves each selection to the lowest usable targets.
It preserves a frame-capable surface as one efficient stream and phases the
surface's elements spatially. Otherwise, it descends through surfaces, groups,
and elements, falling back to the nearest colour-capable ancestor when a
branch has no usable descendant. Collections enter the same expansion after
their nested membership is resolved. Overlapping selections are deduplicated.

`ffmpeg` decodes the file once into signed 16-bit stereo PCM. The example
analyses that exact stream before forwarding it to `ffplay`, so playback and
lighting do not drift through independent decoding. Playback and lighting
updates run concurrently: when a target cannot keep up, the renderer skips
stale spectrum snapshots instead of delaying audio.
