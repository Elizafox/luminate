// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#![allow(
    clippy::print_stdout,
    reason = "This runnable media example reports its final playback statistics."
)]

use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::io::{self, ErrorKind, Read as _, Write as _};
use std::process::{Child, ChildStdout, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use luminate::{Client, Colour, FrameEnvelope, FramePayload, Rgb, TargetId};
use luminate_platform::terminal::escape;
use tokio::time::sleep_until;

const DEVICE_ID: &str = "alienware-keyboard";
const SURFACE_ID: &str = "keyboard";
const SOCKET_PATH_ENV: &str = "LUMINATED_SOCKET_PATH";
// Leave pacing headroom below the keyboard's advertised 15 fps ceiling.
// Scheduling exactly on that boundary makes harmless wake-up jitter look like
// an over-rate frame to the daemon's strict limiter.
const FRAME_RATE: u16 = 12;
const FRAME_WIDTH: usize = 32;
const FRAME_HEIGHT: usize = 11;
const DEFAULT_THRESHOLD: u8 = 112;
const CHROMA_RADIUS_X: usize = 2;
const CHROMA_RADIUS_Y: usize = 1;

type DemoResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone, Copy)]
struct KeySample {
    name: &'static str,
    x: usize,
    y: usize,
}

// Approximate physical centres on the M16 R2 US-ANSI board. Pixels arrive
// from ffmpeg as a deliberately stretched 32x11 image: at this resolution,
// using the whole keyboard is more legible than preserving video aspect ratio.
const KEY_SAMPLES: &[KeySample] = &[
    KeySample {
        name: "escape",
        x: 0,
        y: 0,
    },
    KeySample {
        name: "f1",
        x: 3,
        y: 0,
    },
    KeySample {
        name: "f2",
        x: 5,
        y: 0,
    },
    KeySample {
        name: "f3",
        x: 7,
        y: 0,
    },
    KeySample {
        name: "f4",
        x: 9,
        y: 0,
    },
    KeySample {
        name: "f5",
        x: 12,
        y: 0,
    },
    KeySample {
        name: "f6",
        x: 14,
        y: 0,
    },
    KeySample {
        name: "f7",
        x: 16,
        y: 0,
    },
    KeySample {
        name: "f8",
        x: 18,
        y: 0,
    },
    KeySample {
        name: "f9",
        x: 21,
        y: 0,
    },
    KeySample {
        name: "f10",
        x: 23,
        y: 0,
    },
    KeySample {
        name: "f11",
        x: 25,
        y: 0,
    },
    KeySample {
        name: "f12",
        x: 27,
        y: 0,
    },
    KeySample {
        name: "home",
        x: 27,
        y: 0,
    },
    KeySample {
        name: "end",
        x: 29,
        y: 0,
    },
    KeySample {
        name: "delete",
        x: 31,
        y: 0,
    },
    KeySample {
        name: "mic-mute",
        x: 31,
        y: 2,
    },
    KeySample {
        name: "grave",
        x: 1,
        y: 2,
    },
    KeySample {
        name: "1",
        x: 3,
        y: 2,
    },
    KeySample {
        name: "2",
        x: 5,
        y: 2,
    },
    KeySample {
        name: "3",
        x: 7,
        y: 2,
    },
    KeySample {
        name: "4",
        x: 9,
        y: 2,
    },
    KeySample {
        name: "5",
        x: 11,
        y: 2,
    },
    KeySample {
        name: "6",
        x: 13,
        y: 2,
    },
    KeySample {
        name: "7",
        x: 15,
        y: 2,
    },
    KeySample {
        name: "8",
        x: 17,
        y: 2,
    },
    KeySample {
        name: "9",
        x: 19,
        y: 2,
    },
    KeySample {
        name: "0",
        x: 21,
        y: 2,
    },
    KeySample {
        name: "minus",
        x: 23,
        y: 2,
    },
    KeySample {
        name: "equals",
        x: 25,
        y: 2,
    },
    KeySample {
        name: "backspace",
        x: 28,
        y: 2,
    },
    KeySample {
        name: "volume-mute",
        x: 31,
        y: 4,
    },
    KeySample {
        name: "tab",
        x: 2,
        y: 4,
    },
    KeySample {
        name: "q",
        x: 4,
        y: 4,
    },
    KeySample {
        name: "w",
        x: 6,
        y: 4,
    },
    KeySample {
        name: "e",
        x: 8,
        y: 4,
    },
    KeySample {
        name: "r",
        x: 10,
        y: 4,
    },
    KeySample {
        name: "t",
        x: 12,
        y: 4,
    },
    KeySample {
        name: "y",
        x: 14,
        y: 4,
    },
    KeySample {
        name: "u",
        x: 16,
        y: 4,
    },
    KeySample {
        name: "i",
        x: 18,
        y: 4,
    },
    KeySample {
        name: "o",
        x: 20,
        y: 4,
    },
    KeySample {
        name: "p",
        x: 22,
        y: 4,
    },
    KeySample {
        name: "left-bracket",
        x: 24,
        y: 4,
    },
    KeySample {
        name: "right-bracket",
        x: 26,
        y: 4,
    },
    KeySample {
        name: "backslash",
        x: 28,
        y: 4,
    },
    KeySample {
        name: "volume-down",
        x: 31,
        y: 8,
    },
    KeySample {
        name: "volume-up",
        x: 31,
        y: 6,
    },
    KeySample {
        name: "caps-lock",
        x: 2,
        y: 6,
    },
    KeySample {
        name: "a",
        x: 5,
        y: 6,
    },
    KeySample {
        name: "s",
        x: 7,
        y: 6,
    },
    KeySample {
        name: "d",
        x: 9,
        y: 6,
    },
    KeySample {
        name: "f",
        x: 11,
        y: 6,
    },
    KeySample {
        name: "g",
        x: 13,
        y: 6,
    },
    KeySample {
        name: "h",
        x: 15,
        y: 6,
    },
    KeySample {
        name: "j",
        x: 17,
        y: 6,
    },
    KeySample {
        name: "k",
        x: 19,
        y: 6,
    },
    KeySample {
        name: "l",
        x: 21,
        y: 6,
    },
    KeySample {
        name: "semicolon",
        x: 23,
        y: 6,
    },
    KeySample {
        name: "apostrophe",
        x: 25,
        y: 6,
    },
    KeySample {
        name: "enter",
        x: 28,
        y: 6,
    },
    KeySample {
        name: "left-shift",
        x: 2,
        y: 8,
    },
    KeySample {
        name: "z",
        x: 6,
        y: 8,
    },
    KeySample {
        name: "x",
        x: 8,
        y: 8,
    },
    KeySample {
        name: "c",
        x: 10,
        y: 8,
    },
    KeySample {
        name: "v",
        x: 12,
        y: 8,
    },
    KeySample {
        name: "b",
        x: 14,
        y: 8,
    },
    KeySample {
        name: "n",
        x: 16,
        y: 8,
    },
    KeySample {
        name: "m",
        x: 18,
        y: 8,
    },
    KeySample {
        name: "comma",
        x: 20,
        y: 8,
    },
    KeySample {
        name: "period",
        x: 22,
        y: 8,
    },
    KeySample {
        name: "slash",
        x: 24,
        y: 8,
    },
    KeySample {
        name: "right-shift",
        x: 26,
        y: 8,
    },
    KeySample {
        name: "up",
        x: 29,
        y: 8,
    },
    KeySample {
        name: "left-ctrl",
        x: 1,
        y: 10,
    },
    KeySample {
        name: "fn",
        x: 4,
        y: 10,
    },
    KeySample {
        name: "left-win",
        x: 6,
        y: 10,
    },
    KeySample {
        name: "left-alt",
        x: 8,
        y: 10,
    },
    KeySample {
        name: "space",
        x: 13,
        y: 10,
    },
    KeySample {
        name: "right-win",
        x: 22,
        y: 10,
    },
    KeySample {
        name: "right-alt",
        x: 20,
        y: 10,
    },
    KeySample {
        name: "right-ctrl",
        x: 24,
        y: 10,
    },
    KeySample {
        name: "left",
        x: 27,
        y: 10,
    },
    KeySample {
        name: "down",
        x: 29,
        y: 10,
    },
    KeySample {
        name: "right",
        x: 31,
        y: 10,
    },
];

struct Options {
    media: OsString,
    audio: bool,
    colour: bool,
    invert: bool,
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn options() -> DemoResult<Options> {
    let mut args = env::args_os().skip(1);
    let media = args.next().ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            "usage: luminate-bad-apple-demo <media> [--audio] [--colour] [--invert]",
        )
    })?;
    let mut options = Options {
        media,
        audio: false,
        colour: false,
        invert: false,
    };
    for argument in args {
        match argument.to_str() {
            Some("--audio") => options.audio = true,
            Some("--colour") => options.colour = true,
            Some("--invert") => options.invert = true,
            _ => {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("unknown argument: {}", argument.to_string_lossy()),
                )
                .into());
            }
        }
    }
    Ok(options)
}

fn start_decoder(media: &OsString, colour: bool) -> DemoResult<(ChildGuard, ChildStdout)> {
    let filter = format!("fps={FRAME_RATE},scale={FRAME_WIDTH}:{FRAME_HEIGHT}:flags=area");
    let pixel_format = if colour { "rgb24" } else { "gray" };
    let mut child = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-i"])
        .arg(media)
        .args([
            "-an",
            "-vf",
            &filter,
            "-f",
            "rawvideo",
            "-pix_fmt",
            pixel_format,
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .spawn()?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("ffmpeg did not provide a video pipe"))?;
    Ok((ChildGuard(child), output))
}

fn start_audio(media: &OsString) -> DemoResult<ChildGuard> {
    Command::new("ffplay")
        .args(["-nodisp", "-autoexit", "-loglevel", "error", "-i"])
        .arg(media)
        .stdin(Stdio::null())
        .spawn()
        .map(ChildGuard)
        .map_err(Into::into)
}

fn read_frame(video: &mut ChildStdout, frame: &mut [u8]) -> io::Result<bool> {
    let mut offset = 0;
    while offset < frame.len() {
        let remaining = frame
            .get_mut(offset..)
            .ok_or_else(|| io::Error::other("video frame offset is out of range"))?;
        match video.read(remaining)? {
            0 if offset == 0 => return Ok(false),
            0 => {
                return Err(io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "ffmpeg ended partway through a video frame",
                ));
            }
            count => offset += count,
        }
    }
    Ok(true)
}

fn frame_samples(element_names: &[String]) -> DemoResult<Vec<KeySample>> {
    if KEY_SAMPLES.len() != 85 {
        return Err(io::Error::other("internal M16 R2 sample map is not 85 keys").into());
    }
    element_names
        .iter()
        .map(|name| {
            KEY_SAMPLES
                .iter()
                .find(|sample| sample.name == name)
                .copied()
                .ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::InvalidData,
                        format!("no M16 R2 video sample coordinate for key {name}"),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn monochrome_pixels(frame: &[u8], samples: &[KeySample], invert: bool) -> Vec<Colour> {
    samples
        .iter()
        .map(|sample| {
            let index = (sample.y * FRAME_WIDTH) + sample.x;
            let lit = frame.get(index).copied().unwrap_or(0) >= DEFAULT_THRESHOLD;
            let value = if lit ^ invert { u8::MAX } else { 0 };
            Colour::rgb(Rgb::new(value, value, value))
        })
        .collect()
}

fn rgb_pixel(frame: &[u8], x: usize, y: usize) -> Rgb {
    let offset = ((y * FRAME_WIDTH) + x) * 3;
    Rgb::new(
        frame.get(offset).copied().unwrap_or(0),
        frame.get(offset + 1).copied().unwrap_or(0),
        frame.get(offset + 2).copied().unwrap_or(0),
    )
}

fn luma(rgb: Rgb) -> i32 {
    ((77 * i32::from(rgb.r)) + (150 * i32::from(rgb.g)) + (29 * i32::from(rgb.b)) + 128) / 256
}

fn rounded_dividend(dividend: i32, divisor: i32) -> i32 {
    if dividend >= 0 {
        (dividend + (divisor / 2)) / divisor
    } else {
        (dividend - (divisor / 2)) / divisor
    }
}

fn channel(value: i32) -> u8 {
    u8::try_from(value.clamp(0, 255)).unwrap_or_default()
}

fn colour_pixel(frame: &[u8], sample: KeySample, invert: bool) -> Rgb {
    let centre_luma = luma(rgb_pixel(frame, sample.x, sample.y));
    let x_start = sample.x.saturating_sub(CHROMA_RADIUS_X);
    let x_end = sample
        .x
        .saturating_add(CHROMA_RADIUS_X)
        .min(FRAME_WIDTH - 1);
    let y_start = sample.y.saturating_sub(CHROMA_RADIUS_Y);
    let y_end = sample
        .y
        .saturating_add(CHROMA_RADIUS_Y)
        .min(FRAME_HEIGHT - 1);

    let mut red_difference = 0_i32;
    let mut blue_difference = 0_i32;
    let mut count = 0_i32;
    for y in y_start..=y_end {
        for x in x_start..=x_end {
            let rgb = rgb_pixel(frame, x, y);
            let pixel_luma = luma(rgb);
            red_difference += i32::from(rgb.r) - pixel_luma;
            blue_difference += i32::from(rgb.b) - pixel_luma;
            count += 1;
        }
    }
    if count == 0 {
        return Rgb::BLACK;
    }

    let red = centre_luma + rounded_dividend(red_difference, count);
    let blue = centre_luma + rounded_dividend(blue_difference, count);
    let green = rounded_dividend((256 * centre_luma) - (77 * red) - (29 * blue), 150);
    let (red, green, blue) = if invert {
        (255 - red, 255 - green, 255 - blue)
    } else {
        (red, green, blue)
    };

    Rgb::new(channel(red), channel(green), channel(blue))
}

fn colour_pixels(frame: &[u8], samples: &[KeySample], invert: bool) -> Vec<Colour> {
    samples
        .iter()
        .map(|&sample| Colour::rgb(colour_pixel(frame, sample, invert)))
        .collect()
}

async fn play(options: &Options) -> DemoResult<()> {
    let client = match env::var_os(SOCKET_PATH_ENV) {
        Some(path) => Client::connect_path(path).await?,
        None => Client::connect().await?,
    };
    let device = client
        .list_devices()
        .await?
        .into_iter()
        .find(|device| device.id.as_str() == DEVICE_ID)
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "Alienware keyboard not found"))?;
    let surface = device
        .surfaces
        .iter()
        .find(|surface| surface.id.as_str() == SURFACE_ID)
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "keyboard surface not found"))?;
    let capability = surface.capabilities.frame_upload.as_ref().ok_or_else(|| {
        io::Error::new(
            ErrorKind::Unsupported,
            "keyboard does not advertise frame streaming; rebuild/restart the Alienware plugin",
        )
    })?;
    if capability.max_rate_hz.is_some_and(|rate| rate < FRAME_RATE) {
        return Err(io::Error::new(
            ErrorKind::Unsupported,
            format!("keyboard frame-rate limit is below {FRAME_RATE} fps"),
        )
        .into());
    }

    let element_names = surface
        .elements
        .iter()
        .map(|element| element.id.as_str().to_owned())
        .collect::<Vec<_>>();
    let samples = frame_samples(&element_names)?;
    let target = TargetId::surface(DEVICE_ID, SURFACE_ID);
    let generation = client.begin_frame_stream(target.clone()).await?;
    let result = stream_video(&client, &target, generation, &samples, options).await;
    let end_result = client.end_frame_stream(target, generation).await;

    result?;
    end_result?;
    Ok(())
}

async fn stream_video(
    client: &Client,
    target: &TargetId,
    generation: u32,
    samples: &[KeySample],
    options: &Options,
) -> DemoResult<()> {
    let (mut decoder, mut video) = start_decoder(&options.media, options.colour)?;
    let channels = if options.colour { 3 } else { 1 };
    let mut raw_frame = vec![0_u8; FRAME_WIDTH * FRAME_HEIGHT * channels];
    let mut sequence = 0_u64;
    let mut audio = None;
    let mut started_at: Option<Instant> = None;
    let mut dropped = 0_u64;

    loop {
        if !read_frame(&mut video, &mut raw_frame)? {
            break;
        }

        let start = *started_at.get_or_insert_with(Instant::now);
        if options.audio && audio.is_none() {
            audio = Some(start_audio(&options.media)?);
        }
        let deadline = start
            + Duration::from_nanos(sequence.saturating_mul(1_000_000_000) / u64::from(FRAME_RATE));
        sleep_until(deadline.into()).await;

        let pixels = if options.colour {
            colour_pixels(&raw_frame, samples, options.invert)
        } else {
            monochrome_pixels(&raw_frame, samples, options.invert)
        };
        let acknowledgement = client
            .upload_frame(
                target.clone(),
                FrameEnvelope {
                    generation,
                    sequence,
                    payload: FramePayload::Full(pixels),
                    commit: false,
                },
            )
            .await?;
        dropped += u64::from(acknowledgement.dropped);
        sequence = sequence.saturating_add(1);
    }

    let decoder_status = decoder.0.wait()?;
    if !decoder_status.success() {
        return Err(io::Error::other(format!("ffmpeg exited with {decoder_status}")).into());
    }
    if let Some(mut audio) = audio {
        let _ = audio.0.wait();
    }
    println!("played {sequence} frames ({dropped} rate-limited by the daemon)");
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match options() {
        Ok(options) => match play(&options).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                let _ = writeln!(
                    io::stderr().lock(),
                    "{}",
                    escape(&format!("error: {error}"))
                );
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            let _ = writeln!(
                io::stderr().lock(),
                "{}",
                escape(&format!("error: {error}"))
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CENTRE: KeySample = KeySample {
        name: "test",
        x: FRAME_WIDTH / 2,
        y: FRAME_HEIGHT / 2,
    };

    #[test]
    fn uniform_colour_survives_separate_luma_and_chroma_sampling() {
        let mut frame = vec![0_u8; FRAME_WIDTH * FRAME_HEIGHT * 3];
        for pixel in frame.chunks_exact_mut(3) {
            pixel.copy_from_slice(&[240, 60, 20]);
        }

        let output = colour_pixel(&frame, CENTRE, false);

        assert!(output.r.abs_diff(240) <= 1, "{output:?}");
        assert!(output.g.abs_diff(60) <= 1, "{output:?}");
        assert!(output.b.abs_diff(20) <= 1, "{output:?}");
    }

    #[test]
    fn chroma_is_smoother_than_luma() {
        let mut frame = vec![128_u8; FRAME_WIDTH * FRAME_HEIGHT * 3];
        let offset = ((CENTRE.y * FRAME_WIDTH) + CENTRE.x) * 3;
        frame[offset..offset + 3].copy_from_slice(&[255, 0, 0]);

        let output = colour_pixel(&frame, CENTRE, false);

        assert!(output.r.abs_diff(output.g) < 20, "{output:?}");
        assert!(output.g.abs_diff(output.b) < 20, "{output:?}");
        assert!(luma(output).abs_diff(luma(Rgb::new(255, 0, 0))) <= 1);
    }

    #[test]
    fn colour_inversion_produces_a_negative() {
        let frame = vec![64_u8; FRAME_WIDTH * FRAME_HEIGHT * 3];

        assert_eq!(colour_pixel(&frame, CENTRE, true), Rgb::new(191, 191, 191));
    }
}
