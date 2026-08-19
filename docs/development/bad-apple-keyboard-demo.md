<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Bad Apple on the M16 R2 keyboard

This local demo decodes a user-supplied video with `ffmpeg`, samples it onto
the Alienware M16 R2 US-ANSI key layout, and sends volatile full frames through
`libluminate`. It does not persist lighting state to the keyboard's firmware.

Build the updated daemon and Alienware plugin, start `luminated` with that
plugin, then run from the repository root:

```sh
LUMINATED_SOCKET_PATH=/tmp/luminated-alienware/luminated.sock \
  cargo run --release -p libluminate --example bad_apple -- \
  /path/to/video.webm --audio
```

Omit `LUMINATED_SOCKET_PATH` when the daemon uses Luminate's default system
socket.

`--audio` starts `ffplay` alongside the first decoded frame. Omit it for silent
playback. By default, the demo thresholds the video into the original stark
black-and-white look. Add `--colour` for continuous RGB output:

```sh
cargo run --release -p libluminate --example bad_apple -- \
  /path/to/video.webm --audio --colour
```

Colour mode keeps the luminance sampled at each key's centre, but averages the
two chroma components over a wider 5-by-3-pixel neighbourhood. This preserves
more of the video's edge detail while making colour less prone to flicker at
the keyboard's very low effective resolution. It is only an approximation:
there are 85 unevenly spaced LEDs, the key legends diffuse and tint their
light, out-of-gamut reconstructed colours are clipped, and playback is limited
to 12 frames per second.

Use `--invert` if the monochrome source has the opposite black/white polarity.
In colour mode it produces a photographic negative. The demo requires
`ffmpeg` and, for audio, `ffplay` on `PATH`.
