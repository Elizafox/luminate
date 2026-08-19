// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Plays an audio file and renders its spectrum on selected Luminate targets.
//!
//! `ffmpeg` decodes one PCM stream. The example analyses those samples before
//! forwarding them to `ffplay`, keeping the lights tied to the audio actually
//! sent to the computer's output device.

#![allow(
    clippy::print_stdout,
    reason = "this interactive example reports its selected targets and playback statistics"
)]

use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::f32::consts::TAU;
use std::ffi::OsString;
use std::io::{self, ErrorKind, Write as _};
use std::process::{ExitCode, ExitStatus, Stdio};
use std::time::Duration;

use luminate::capability::{CapabilitySet, ColourCapability, ColourChannel};
use luminate::collection::CollectionGraph;
use luminate::colour::ColourChannelValue;
use luminate::{
    Client, CollectionId, Colour, Device, Effect, ElementGeometry, FrameEnvelope, FramePayload,
    GroupMember, Rgb, TargetId,
};
use luminate_platform::terminal::escape;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::watch;
use tokio::time::{MissedTickBehavior, interval};

const SOCKET_PATH_ENV: &str = "LUMINATED_SOCKET_PATH";
const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: usize = 2;
const FRAME_RATE: u32 = 20;
const SAMPLES_PER_FRAME: usize = SAMPLE_RATE as usize / FRAME_RATE as usize;
const BYTES_PER_SAMPLE: usize = 2;
const PCM_FRAME_BYTES: usize = SAMPLES_PER_FRAME * CHANNELS * BYTES_PER_SAMPLE;
const BAND_FREQUENCIES: [f32; 12] = [
    60.0, 90.0, 135.0, 200.0, 300.0, 450.0, 675.0, 1_000.0, 1_500.0, 2_250.0, 3_400.0, 5_100.0,
];

type DemoResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Selection {
    Target(TargetId),
    Collection(CollectionId),
}

#[derive(Debug)]
struct Options {
    media: OsString,
    selections: Vec<Selection>,
}

#[derive(Clone)]
enum StaticModel {
    Additive(Vec<(ColourChannel, u8)>),
    Hsv {
        hue_bits: u8,
        saturation_bits: u8,
        value_bits: u8,
    },
    Hsl {
        hue_bits: u8,
        saturation_bits: u8,
        lightness_bits: u8,
    },
    Monochrome {
        bits: u8,
    },
}

enum RenderMode {
    Frame {
        generation: u32,
        positions: Vec<f32>,
    },
    Static(StaticModel),
}

struct RenderTarget {
    target: TargetId,
    mode: RenderMode,
    phase: f32,
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.start_kill();
    }
}

fn usage_error(message: impl Into<String>) -> Box<dyn Error> {
    io::Error::new(
        ErrorKind::InvalidInput,
        format!(
            "{}\nusage: audio_visualizer <audio-file> \\\n+  [--device ID] [--surface DEVICE/SURFACE] [--group DEVICE/GROUP] \\\n+  [--collection ID] ...",
            message.into()
        ),
    )
    .into()
}

fn split_pair(value: &OsString, flag: &str) -> DemoResult<(String, String)> {
    let value = value
        .to_str()
        .ok_or_else(|| usage_error(format!("{flag} must be UTF-8")))?;
    let (left, right) = value
        .split_once('/')
        .ok_or_else(|| usage_error(format!("{flag} requires DEVICE/ID")))?;
    if left.is_empty() || right.is_empty() || right.contains('/') {
        return Err(usage_error(format!("{flag} requires DEVICE/ID")));
    }

    Ok((left.to_owned(), right.to_owned()))
}

fn options_from(arguments: impl IntoIterator<Item = OsString>) -> DemoResult<Options> {
    let mut arguments = arguments.into_iter();
    let media = arguments
        .next()
        .ok_or_else(|| usage_error("an audio file is required"))?;
    let mut selections = Vec::new();

    while let Some(flag) = arguments.next() {
        let value = arguments
            .next()
            .ok_or_else(|| usage_error(format!("{} requires a value", flag.to_string_lossy())))?;
        match flag.to_str() {
            Some("--device") => selections.push(Selection::Target(TargetId::device(
                value
                    .into_string()
                    .map_err(|_| usage_error("--device must be UTF-8"))?,
            ))),
            Some("--surface") => {
                let (device, surface) = split_pair(&value, "--surface")?;
                selections.push(Selection::Target(TargetId::surface(device, surface)));
            }
            Some("--group") => {
                let (device, group) = split_pair(&value, "--group")?;
                selections.push(Selection::Target(TargetId::group(device, group)));
            }
            Some("--collection") => selections.push(Selection::Collection(CollectionId::new(
                value
                    .into_string()
                    .map_err(|_| usage_error("--collection must be UTF-8"))?,
            ))),
            _ => {
                return Err(usage_error(format!(
                    "unknown argument: {}",
                    flag.to_string_lossy()
                )));
            }
        }
    }
    if selections.is_empty() {
        return Err(usage_error("at least one target or collection is required"));
    }

    Ok(Options { media, selections })
}

fn options() -> DemoResult<Options> {
    options_from(env::args_os().skip(1))
}

fn resolve_targets(
    selections: &[Selection],
    collections: &[luminate::Collection],
) -> DemoResult<Vec<TargetId>> {
    let graph = CollectionGraph::new(collections);
    let mut seen = HashSet::new();
    let mut targets = Vec::new();

    for selection in selections {
        let selected = match selection {
            Selection::Target(target) => vec![target.clone()],
            Selection::Collection(id) => graph.resolve_leaves(id)?,
        };
        for target in selected {
            if seen.insert(target.clone()) {
                targets.push(target);
            }
        }
    }

    Ok(targets)
}

fn target_capabilities<'a>(devices: &'a [Device], target: &TargetId) -> Option<&'a CapabilitySet> {
    let device = devices
        .iter()
        .find(|device| &device.id == target.device_id())?;
    match target {
        TargetId::Device(_) => Some(&device.capabilities),
        TargetId::Surface { surface, .. } => device
            .surfaces
            .iter()
            .find(|candidate| &candidate.id == surface)
            .map(|surface| &surface.capabilities),
        TargetId::Element {
            surface, element, ..
        } => device
            .surfaces
            .iter()
            .find(|candidate| &candidate.id == surface)?
            .elements
            .iter()
            .find(|candidate| &candidate.id == element)
            .map(|element| &element.capabilities),
        TargetId::Group { group, .. } => device
            .groups
            .iter()
            .find(|candidate| &candidate.id == group)
            .map(|group| &group.capabilities),
    }
}

fn target_can_render(devices: &[Device], target: &TargetId) -> bool {
    target_capabilities(devices, target).is_some_and(|capabilities| {
        (capabilities.frame_upload.is_some() && surface_positions(devices, target).is_some())
            || static_model(capabilities).is_some()
    })
}

fn expand_targets(devices: &[Device], targets: Vec<TargetId>) -> DemoResult<Vec<TargetId>> {
    let mut expanded = Vec::new();
    let mut seen = HashSet::new();

    for target in targets {
        let mut visiting_groups = HashSet::new();
        expand_target(
            devices,
            &target,
            &mut visiting_groups,
            &mut seen,
            &mut expanded,
        )?;
    }

    Ok(expanded)
}

fn expand_target(
    devices: &[Device],
    target: &TargetId,
    visiting_groups: &mut HashSet<TargetId>,
    seen: &mut HashSet<TargetId>,
    expanded: &mut Vec<TargetId>,
) -> DemoResult<bool> {
    let device = devices
        .iter()
        .find(|device| &device.id == target.device_id())
        .ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                format!("selected target's device does not exist: {target:?}"),
            )
        })?;
    let descendants_render = match &target {
        TargetId::Device(_) => {
            let mut rendered = false;
            for surface in &device.surfaces {
                rendered |= expand_target(
                    devices,
                    &TargetId::surface(device.id.as_str(), surface.id.as_str()),
                    visiting_groups,
                    seen,
                    expanded,
                )?;
            }
            rendered
        }
        TargetId::Surface { surface, .. } => {
            let surface = device
                .surfaces
                .iter()
                .find(|candidate| &candidate.id == surface)
                .ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::NotFound,
                        format!("selected surface does not exist: {target:?}"),
                    )
                })?;
            let mut rendered = false;
            if surface.capabilities.frame_upload.is_none() {
                for element in &surface.elements {
                    rendered |= expand_target(
                        devices,
                        &TargetId::element(
                            device.id.as_str(),
                            surface.id.as_str(),
                            element.id.as_str(),
                        ),
                        visiting_groups,
                        seen,
                        expanded,
                    )?;
                }
            }
            rendered
        }
        TargetId::Element { .. } => false,
        TargetId::Group { group, .. } => {
            if !visiting_groups.insert(target.clone()) {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    format!("group membership cycle reaches {target:?}"),
                )
                .into());
            }
            let group = device
                .groups
                .iter()
                .find(|candidate| &candidate.id == group)
                .ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::NotFound,
                        format!("selected group does not exist: {target:?}"),
                    )
                })?;
            let mut rendered = false;
            for member in &group.members {
                let member = match member {
                    GroupMember::Surface(surface) => {
                        TargetId::surface(device.id.as_str(), surface.as_str())
                    }
                    GroupMember::Element { surface, element } => {
                        TargetId::element(device.id.as_str(), surface.as_str(), element.as_str())
                    }
                    GroupMember::Group(group) => {
                        TargetId::group(device.id.as_str(), group.as_str())
                    }
                };
                rendered |= expand_target(devices, &member, visiting_groups, seen, expanded)?;
            }
            visiting_groups.remove(target);
            rendered
        }
    };

    let target_renders = !descendants_render && target_can_render(devices, target);
    if target_renders && seen.insert(target.clone()) {
        expanded.push(target.clone());
    }

    Ok(descendants_render || target_renders)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "surface element counts are small hardware topology values"
)]
fn surface_positions(devices: &[Device], target: &TargetId) -> Option<Vec<f32>> {
    let TargetId::Surface {
        device: device_id,
        surface: surface_id,
    } = target
    else {
        return None;
    };
    let surface = devices
        .iter()
        .find(|device| &device.id == device_id)?
        .surfaces
        .iter()
        .find(|surface| &surface.id == surface_id)?;
    let denominator = surface.elements.len().saturating_sub(1).max(1) as f32;

    Some(
        surface
            .elements
            .iter()
            .enumerate()
            .map(|(index, element)| match element.geometry {
                Some(ElementGeometry::Rect { x, w, .. }) => (x + (w / 2.0)).clamp(0.0, 1.0),
                Some(ElementGeometry::Point { x, .. }) => x.clamp(0.0, 1.0),
                Some(ElementGeometry::Linear { position }) => position.clamp(0.0, 1.0),
                Some(ElementGeometry::MatrixCell { col, .. }) => {
                    let largest_column = surface
                        .elements
                        .iter()
                        .filter_map(|element| match element.geometry {
                            Some(ElementGeometry::MatrixCell { col, .. }) => Some(col),
                            _ => None,
                        })
                        .max()
                        .unwrap_or(1)
                        .max(1);
                    f32::from(col) / f32::from(largest_column)
                }
                None => index as f32 / denominator,
            })
            .collect(),
    )
}

fn static_model(capabilities: &CapabilitySet) -> Option<StaticModel> {
    capabilities
        .colour
        .iter()
        .find_map(|capability| match capability {
            ColourCapability::Additive(channels) => {
                let useful = channels
                    .iter()
                    .filter(|channel| {
                        matches!(
                            channel.channel,
                            ColourChannel::Red
                                | ColourChannel::Green
                                | ColourChannel::Blue
                                | ColourChannel::White
                                | ColourChannel::WarmWhite
                                | ColourChannel::CoolWhite
                        )
                    })
                    .map(|channel| (channel.channel, channel.bits))
                    .collect::<Vec<_>>();
                (!useful.is_empty()).then_some(StaticModel::Additive(useful))
            }
            ColourCapability::Hsv {
                hue_bits,
                saturation_bits,
                value_bits,
            } => Some(StaticModel::Hsv {
                hue_bits: *hue_bits,
                saturation_bits: *saturation_bits,
                value_bits: *value_bits,
            }),
            ColourCapability::Hsl {
                hue_bits,
                saturation_bits,
                lightness_bits,
            } => Some(StaticModel::Hsl {
                hue_bits: *hue_bits,
                saturation_bits: *saturation_bits,
                lightness_bits: *lightness_bits,
            }),
            ColourCapability::Monochrome { bits } => Some(StaticModel::Monochrome { bits: *bits }),
            ColourCapability::Cct { .. } => None,
        })
}

fn maximum(bits: u8) -> u32 {
    match bits {
        0 => 0,
        1..=31 => (1_u32 << bits) - 1,
        32..=u8::MAX => u32::MAX,
    }
}

fn scale_channel(value: u8, bits: u8) -> u32 {
    u32::from(value).saturating_mul(maximum(bits)) / u32::from(u8::MAX)
}

#[allow(
    clippy::float_cmp,
    reason = "largest is selected directly from these exact channel values"
)]
fn rgb_to_hsx(rgb: Rgb) -> (f32, f32, f32, f32) {
    let red = f32::from(rgb.r) / 255.0;
    let green = f32::from(rgb.g) / 255.0;
    let blue = f32::from(rgb.b) / 255.0;
    let largest = red.max(green).max(blue);
    let smallest = red.min(green).min(blue);
    let delta = largest - smallest;
    let hue = if delta <= f32::EPSILON {
        0.0
    } else if largest == red {
        ((green - blue) / delta).rem_euclid(6.0) / 6.0
    } else if largest == green {
        (((blue - red) / delta) + 2.0) / 6.0
    } else {
        (((red - green) / delta) + 4.0) / 6.0
    };
    let value_saturation = if largest <= f32::EPSILON {
        0.0
    } else {
        delta / largest
    };
    let lightness = largest.midpoint(smallest);
    let lightness_saturation = if delta <= f32::EPSILON {
        0.0
    } else {
        delta / (1.0 - ((2.0 * lightness) - 1.0).abs())
    };

    (hue, value_saturation, largest, lightness_saturation)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "the finite value is clamped before conversion to the advertised integer range"
)]
fn scale_unit(value: f32, bits: u8) -> u32 {
    (value.clamp(0.0, 1.0) * maximum(bits) as f32).round() as u32
}

fn static_colour(model: &StaticModel, rgb: Rgb) -> DemoResult<Colour> {
    match model {
        StaticModel::Additive(channels) => {
            let luminance =
                ((54 * u32::from(rgb.r)) + (183 * u32::from(rgb.g)) + (19 * u32::from(rgb.b)))
                    / 256;
            let values = channels
                .iter()
                .map(|&(channel, bits)| {
                    let value = match channel {
                        ColourChannel::Red => rgb.r,
                        ColourChannel::Green => rgb.g,
                        ColourChannel::Blue => rgb.b,
                        ColourChannel::White
                        | ColourChannel::WarmWhite
                        | ColourChannel::CoolWhite => u8::try_from(luminance).unwrap_or(u8::MAX),
                        ColourChannel::Amber
                        | ColourChannel::Ultraviolet
                        | ColourChannel::Hue
                        | ColourChannel::Saturation
                        | ColourChannel::Value
                        | ColourChannel::Lightness
                        | ColourChannel::Temperature
                        | ColourChannel::Intensity => 0,
                    };
                    ColourChannelValue::new(channel, scale_channel(value, bits))
                })
                .collect();
            Ok(Colour::additive(values)?)
        }
        StaticModel::Hsv {
            hue_bits,
            saturation_bits,
            value_bits,
        } => {
            let (hue, saturation, value, _) = rgb_to_hsx(rgb);
            Ok(Colour::hsv(
                scale_unit(hue, *hue_bits),
                scale_unit(saturation, *saturation_bits),
                scale_unit(value, *value_bits),
            ))
        }
        StaticModel::Hsl {
            hue_bits,
            saturation_bits,
            lightness_bits,
        } => {
            let (hue, _, value, saturation) = rgb_to_hsx(rgb);
            let smallest = f32::from(rgb.r.min(rgb.g).min(rgb.b)) / 255.0;
            let lightness = value.midpoint(smallest);
            Ok(Colour::hsl(
                scale_unit(hue, *hue_bits),
                scale_unit(saturation, *saturation_bits),
                scale_unit(lightness, *lightness_bits),
            ))
        }
        StaticModel::Monochrome { bits } => {
            let luminance =
                ((54 * u32::from(rgb.r)) + (183 * u32::from(rgb.g)) + (19 * u32::from(rgb.b)))
                    / 256;
            Ok(Colour::monochrome(
                luminance.saturating_mul(maximum(*bits)) / 255,
            ))
        }
    }
}

fn mono_samples(pcm: &[u8]) -> Vec<f32> {
    pcm.chunks_exact(CHANNELS * BYTES_PER_SAMPLE)
        .filter_map(|frame| match frame {
            [left_low, left_high, right_low, right_high] => {
                let left = i16::from_le_bytes([*left_low, *left_high]);
                let right = i16::from_le_bytes([*right_low, *right_high]);
                Some((f32::from(left) + f32::from(right)) / (2.0 * f32::from(i16::MAX)))
            }
            _ => None,
        })
        .collect()
}

#[allow(
    clippy::cast_precision_loss,
    reason = "audio windows and the fixed sample rate are small DSP quantities"
)]
fn spectrum(samples: &[f32]) -> [f32; BAND_FREQUENCIES.len()] {
    let mut bands = [0.0; BAND_FREQUENCIES.len()];
    if samples.is_empty() {
        return bands;
    }

    for (band, frequency) in bands.iter_mut().zip(BAND_FREQUENCIES) {
        let coefficient = 2.0 * (TAU * frequency / SAMPLE_RATE as f32).cos();
        let mut previous = 0.0;
        let mut previous_previous = 0.0;
        for &sample in samples {
            let current = sample + coefficient.mul_add(previous, -previous_previous);
            previous_previous = previous;
            previous = current;
        }
        let power = previous_previous.mul_add(
            previous_previous,
            previous.mul_add(previous, -coefficient * previous * previous_previous),
        );
        *band = ((power.max(0.0).sqrt() / samples.len() as f32) * 18.0)
            .sqrt()
            .clamp(0.0, 1.0);
    }

    bands
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "clamped spectrum coordinates are bounded by the twelve-element band array"
)]
fn visualizer_rgb(position: f32, bands: &[f32], frame: u64) -> Rgb {
    let band_position = position.clamp(0.0, 1.0) * (bands.len().saturating_sub(1)) as f32;
    let lower = band_position.floor() as usize;
    let upper = lower.saturating_add(1).min(bands.len().saturating_sub(1));
    let blend = band_position - lower as f32;
    let lower_value = bands.get(lower).copied().unwrap_or_default();
    let upper_value = bands.get(upper).copied().unwrap_or_default();
    let magnitude = lower_value.mul_add(1.0 - blend, upper_value * blend);
    let hue = (position + (frame as f32 * 0.006)).fract();
    hsv_rgb(hue, 0.9, magnitude)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the hue sector is reduced to the finite range zero through five"
)]
fn hsv_rgb(hue: f32, saturation: f32, value: f32) -> Rgb {
    let sector = (hue.rem_euclid(1.0) * 6.0).floor() as u8;
    let fraction = (hue * 6.0).fract();
    let low = value * (1.0 - saturation);
    let descending = value * (1.0 - (saturation * fraction));
    let ascending = value * (1.0 - (saturation * (1.0 - fraction)));
    let (red, green, blue) = match sector {
        0 => (value, ascending, low),
        1 => (descending, value, low),
        2 => (low, value, ascending),
        3 => (low, descending, value),
        4 => (ascending, low, value),
        _ => (value, low, descending),
    };
    Rgb::new(to_channel(red), to_channel(green), to_channel(blue))
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the finite channel is clamped to the complete u8 range"
)]
fn to_channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn start_media(media: &OsString) -> DemoResult<(ChildGuard, ChildStdout, ChildGuard, ChildStdin)> {
    let mut decoder = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-i"])
        .arg(media)
        .args([
            "-vn",
            "-f",
            "s16le",
            "-acodec",
            "pcm_s16le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            &CHANNELS.to_string(),
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let decoder_output = decoder
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("ffmpeg did not provide a PCM pipe"))?;

    let mut player = Command::new("ffplay")
        .args([
            "-nodisp",
            "-autoexit",
            "-loglevel",
            "error",
            "-f",
            "s16le",
            "-sample_rate",
            &SAMPLE_RATE.to_string(),
            "-ch_layout",
            "stereo",
            "-i",
            "pipe:0",
        ])
        .stdin(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let player_input = player
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("ffplay did not provide an audio pipe"))?;

    Ok((
        ChildGuard(decoder),
        decoder_output,
        ChildGuard(player),
        player_input,
    ))
}

#[allow(
    clippy::cast_precision_loss,
    reason = "the number of selected lighting targets is a small topology value"
)]
async fn prepare_targets(
    client: &Client,
    devices: &[Device],
    targets: Vec<TargetId>,
) -> DemoResult<Vec<RenderTarget>> {
    let target_count = targets.len().max(1);
    let mut prepared = Vec::with_capacity(targets.len());

    for (index, target) in targets.into_iter().enumerate() {
        let capabilities = target_capabilities(devices, &target).ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                format!("selected target does not exist: {target:?}"),
            )
        })?;
        if capabilities.frame_upload.is_some()
            && let Some(positions) = surface_positions(devices, &target)
        {
            if positions.is_empty() {
                return Err(io::Error::new(
                    ErrorKind::Unsupported,
                    format!("frame-capable surface has no elements: {target:?}"),
                )
                .into());
            }
            let generation = client.begin_frame_stream(target.clone()).await?;
            prepared.push(RenderTarget {
                target,
                mode: RenderMode::Frame {
                    generation,
                    positions,
                },
                phase: index as f32 / target_count as f32,
            });
        } else if let Some(model) = static_model(capabilities) {
            prepared.push(RenderTarget {
                target,
                mode: RenderMode::Static(model),
                phase: index as f32 / target_count as f32,
            });
        } else {
            return Err(io::Error::new(
                ErrorKind::Unsupported,
                format!("selected target cannot render visualizer colours: {target:?}"),
            )
            .into());
        }
    }

    Ok(prepared)
}

async fn render(
    client: &Client,
    targets: &[RenderTarget],
    bands: &[f32],
    sequence: u64,
) -> DemoResult<u64> {
    let mut dropped = 0;
    for target in targets {
        match &target.mode {
            RenderMode::Frame {
                generation,
                positions,
            } => {
                let pixels = positions
                    .iter()
                    .map(|&position| {
                        Colour::rgb(visualizer_rgb(
                            (position + target.phase).fract(),
                            bands,
                            sequence,
                        ))
                    })
                    .collect();
                let acknowledgement = client
                    .upload_frame(
                        target.target.clone(),
                        FrameEnvelope {
                            generation: *generation,
                            sequence,
                            payload: FramePayload::Full(pixels),
                            commit: false,
                        },
                    )
                    .await?;
                dropped += u64::from(acknowledgement.dropped);
            }
            RenderMode::Static(model) => {
                let colour = visualizer_rgb(target.phase, bands, sequence);
                client
                    .set_effect(
                        target.target.clone(),
                        Effect::Static {
                            colour: static_colour(model, colour)?,
                        },
                    )
                    .await?;
            }
        }
    }

    Ok(dropped)
}

async fn finish_targets(client: &Client, targets: &[RenderTarget]) -> DemoResult<()> {
    for target in targets {
        if let RenderMode::Frame { generation, .. } = target.mode {
            client
                .end_frame_stream(target.target.clone(), generation)
                .await?;
        }
        client.restore_appearance(target.target.clone()).await?;
    }

    Ok(())
}

async fn play_media(
    mut decoder: ChildGuard,
    mut decoder_output: ChildStdout,
    mut player: ChildGuard,
    mut player_input: ChildStdin,
    spectrum_tx: watch::Sender<[f32; BAND_FREQUENCIES.len()]>,
) -> DemoResult<(ExitStatus, ExitStatus)> {
    let mut pcm = vec![0_u8; PCM_FRAME_BYTES];
    let mut smoothed = [0.0; BAND_FREQUENCIES.len()];

    loop {
        let mut read = 0;
        while read < pcm.len() {
            let remaining = pcm
                .get_mut(read..)
                .ok_or_else(|| io::Error::other("PCM read offset is out of range"))?;
            let count = decoder_output.read(remaining).await?;
            if count == 0 {
                break;
            }
            read += count;
        }
        if read == 0 {
            break;
        }
        let complete = read - (read % (CHANNELS * BYTES_PER_SAMPLE));
        let complete_pcm = pcm
            .get(..complete)
            .ok_or_else(|| io::Error::other("PCM frame length is out of range"))?;
        player_input.write_all(complete_pcm).await?;
        let current = spectrum(&mono_samples(complete_pcm));
        for (smooth, value) in smoothed.iter_mut().zip(current) {
            *smooth = value.mul_add(0.45, *smooth * 0.55);
        }
        spectrum_tx.send_replace(smoothed);
        if read < pcm.len() {
            break;
        }
    }

    player_input.shutdown().await?;
    let decoder_status = decoder.0.wait().await?;
    let player_status = player.0.wait().await?;

    Ok((decoder_status, player_status))
}

async fn render_spectra(
    client: &Client,
    targets: &[RenderTarget],
    mut spectrum_rx: watch::Receiver<[f32; BAND_FREQUENCIES.len()]>,
) -> DemoResult<(u64, u64)> {
    let mut sequence = 0_u64;
    let mut dropped = 0_u64;
    let mut ticker = interval(Duration::from_secs_f64(1.0 / f64::from(FRAME_RATE)));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let bands = *spectrum_rx.borrow_and_update();
                dropped += render(client, targets, &bands, sequence).await?;
                sequence = sequence.saturating_add(1);
            }
            changed = spectrum_rx.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
    }

    Ok((sequence, dropped))
}

async fn play(options: &Options) -> DemoResult<()> {
    let client = match env::var_os(SOCKET_PATH_ENV) {
        Some(path) => Client::connect_path(path).await?,
        None => Client::connect().await?,
    };
    let collections = client.list_collections().await?;
    let selected = resolve_targets(&options.selections, &collections)?;
    let devices = client.list_devices().await?;
    let expanded = expand_targets(&devices, selected)?;
    if expanded.is_empty() {
        return Err(io::Error::new(
            ErrorKind::Unsupported,
            "none of the selected targets or their descendants can render the visualizer",
        )
        .into());
    }
    let targets = prepare_targets(&client, &devices, expanded).await?;
    println!("visualizing on {} target(s)", targets.len());

    let (decoder, decoder_output, player, player_input) = start_media(&options.media)?;
    let (spectrum_tx, spectrum_rx) = watch::channel([0.0; BAND_FREQUENCIES.len()]);
    let playback = play_media(decoder, decoder_output, player, player_input, spectrum_tx);
    let rendering = render_spectra(&client, &targets, spectrum_rx);
    let ((decoder_status, player_status), (sequence, dropped)) =
        tokio::try_join!(playback, rendering)?;
    let finish_result = finish_targets(&client, &targets).await;
    if !decoder_status.success() {
        return Err(io::Error::other(format!("ffmpeg exited with {decoder_status}")).into());
    }
    if !player_status.success() {
        return Err(io::Error::other(format!("ffplay exited with {player_status}")).into());
    }
    finish_result?;
    println!("rendered {sequence} frames ({dropped} daemon-rate-limited uploads)");

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
    use luminate::capability::{
        BufferingMode, CapabilityScope, FrameUpdateMode, FrameUploadCapability,
    };
    use luminate::collection::OwnerIdentity;
    use luminate::{
        Collection, CollectionMember, DeviceId, Element, ElementId, ElementKind, Surface,
        SurfaceId, SurfaceKind,
    };

    fn rgb_capabilities() -> CapabilitySet {
        CapabilitySet {
            colour: vec![ColourCapability::rgb8()],
            ..CapabilitySet::default()
        }
    }

    fn element(id: &str) -> Element {
        Element {
            id: ElementId::new(id),
            name: None,
            kind: ElementKind::Led,
            geometry: None,
            physical_tags: Vec::new(),
            capabilities: rgb_capabilities(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn test_device() -> Device {
        let mut frame_capabilities = rgb_capabilities();
        frame_capabilities.frame_upload = Some(FrameUploadCapability {
            scope: CapabilityScope::Surface,
            update_mode: FrameUpdateMode::FullFrameOnly,
            max_rate_hz: Some(20),
            atomic: true,
            buffering: BufferingMode::Immediate,
            shm: None,
        });

        Device {
            id: DeviceId::new("lights"),
            name: "Lights".to_owned(),
            vendor: None,
            model: None,
            provider_instance: None,
            surfaces: vec![
                Surface {
                    id: SurfaceId::new("streaming"),
                    name: "Streaming".to_owned(),
                    kind: SurfaceKind::Linear { length: 1.0 },
                    physical_tags: Vec::new(),
                    elements: vec![element("left"), element("right")],
                    capabilities: frame_capabilities,
                    notes: Vec::new(),
                    warnings: Vec::new(),
                },
                Surface {
                    id: SurfaceId::new("zones"),
                    name: "Zones".to_owned(),
                    kind: SurfaceKind::Opaque,
                    physical_tags: Vec::new(),
                    elements: vec![element("top"), element("bottom")],
                    capabilities: rgb_capabilities(),
                    notes: Vec::new(),
                    warnings: Vec::new(),
                },
            ],
            groups: Vec::new(),
            capabilities: rgb_capabilities(),
            category: None,
            physical_tags: Vec::new(),
            host_attached: false,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn parses_mixed_selections() {
        let options = options_from([
            OsString::from("song.flac"),
            OsString::from("--device"),
            OsString::from("lamp"),
            OsString::from("--surface"),
            OsString::from("keyboard/keys"),
            OsString::from("--group"),
            OsString::from("case/fans"),
            OsString::from("--collection"),
            OsString::from("desk"),
        ])
        .expect("valid options");

        assert_eq!(options.selections.len(), 4);
        assert_eq!(
            options.selections[1],
            Selection::Target(TargetId::surface("keyboard", "keys"))
        );
    }

    #[test]
    fn rejects_a_missing_selection() {
        let error = options_from([OsString::from("song.flac")]).expect_err("selection required");
        assert!(error.to_string().contains("at least one target"));
    }

    #[test]
    fn resolves_collections_and_deduplicates_targets() {
        let lamp = TargetId::device("lamp");
        let collection = Collection {
            id: CollectionId::new("desk"),
            name: "Desk".to_owned(),
            description: None,
            owner: OwnerIdentity::Uid(1000),
            kind: None,
            members: vec![CollectionMember::Target(lamp.clone())],
        };
        let resolved = resolve_targets(
            &[
                Selection::Target(lamp.clone()),
                Selection::Collection(CollectionId::new("desk")),
            ],
            &[collection],
        )
        .expect("collection resolves");

        assert_eq!(resolved, vec![lamp]);
    }

    #[test]
    fn device_expansion_prefers_frame_surfaces_and_deep_static_elements() {
        let expanded =
            expand_targets(&[test_device()], vec![TargetId::device("lights")]).expect("expands");

        assert_eq!(
            expanded,
            vec![
                TargetId::surface("lights", "streaming"),
                TargetId::element("lights", "zones", "top"),
                TargetId::element("lights", "zones", "bottom"),
            ]
        );
    }

    #[test]
    fn overlapping_expansions_do_not_fall_back_to_an_ancestor() {
        let expanded = expand_targets(
            &[test_device()],
            vec![
                TargetId::surface("lights", "streaming"),
                TargetId::device("lights"),
            ],
        )
        .expect("expands");

        assert_eq!(
            expanded,
            vec![
                TargetId::surface("lights", "streaming"),
                TargetId::element("lights", "zones", "top"),
                TargetId::element("lights", "zones", "bottom"),
            ]
        );
    }

    #[test]
    fn silence_has_no_spectrum_energy() {
        assert_eq!(spectrum(&vec![0.0; SAMPLES_PER_FRAME]), [0.0; 12]);
    }

    #[test]
    fn matching_tone_peaks_near_its_band() {
        let samples = (0..SAMPLES_PER_FRAME)
            .map(|index| (TAU * 450.0 * index as f32 / SAMPLE_RATE as f32).sin() * 0.8)
            .collect::<Vec<_>>();
        let bands = spectrum(&samples);
        let peak = bands
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index);

        assert_eq!(peak, Some(5));
    }
}
