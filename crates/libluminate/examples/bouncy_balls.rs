// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Runs an interactive, volatile bouncy-ball simulation on an Alienware M16 R2
//! US-ANSI keyboard.

#![allow(
    clippy::print_stdout,
    reason = "This interactive example reports its final playback statistics."
)]

use std::error::Error;
use std::io::{self, Write as _};
use std::process::ExitCode;

use luminate_platform::terminal::escape;

fn report(result: Result<(), Box<dyn Error>>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
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

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    use std::io::{Error as IoError, ErrorKind};

    report(Err(IoError::new(
        ErrorKind::Unsupported,
        "the bouncy-balls keyboard demo requires Linux evdev",
    )
    .into()))
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeSet;
    use std::env;
    use std::error::Error;
    use std::f64::consts::TAU;
    use std::ffi::OsString;
    use std::io::{self, ErrorKind, IsTerminal as _, Write as _};
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use evdev::{Device, EventStream, EventSummary, KeyCode};
    use luminate::{Client, Colour, FrameEnvelope, FramePayload, Rgb, TargetId};
    use tokio::time::{MissedTickBehavior, interval};

    const DEVICE_ID: &str = "alienware-keyboard";
    const SURFACE_ID: &str = "keyboard";
    const SOCKET_PATH_ENV: &str = "LUMINATED_SOCKET_PATH";

    const FRAME_RATE: u16 = 12;
    const PHYSICS_RATE: u16 = 120;
    const PHYSICS_STEPS_PER_FRAME: u16 = PHYSICS_RATE / FRAME_RATE;
    const WORLD_WIDTH: f64 = 16.0;
    const WORLD_HEIGHT: f64 = 6.0;
    const GRID_WIDTH: usize = 64;
    const GRID_HEIGHT: usize = 24;
    const BALL_RADIUS: f64 = 0.42;
    const BALL_SPEED: f64 = 3.2;
    const FINGER_RADIUS: f64 = 0.34;
    const MAX_BALLS: usize = 10;
    const DEFAULT_BALLS: usize = 5;
    const MAX_SPAWN_ATTEMPTS: usize = 1_000;
    const TERMINAL_ENTER: &[u8] = b"\x1b[?1049h\x1b[2J\x1b[H\x1b[?25l";
    const TERMINAL_LEAVE: &[u8] = b"\x1b[?25h\x1b[0m\x1b[?1049l";

    type DemoResult<T> = Result<T, Box<dyn Error>>;

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Vector {
        x: f64,
        y: f64,
    }

    impl Vector {
        const fn new(x: f64, y: f64) -> Self {
            Self { x, y }
        }

        fn add(self, other: Self) -> Self {
            Self::new(self.x + other.x, self.y + other.y)
        }

        fn subtract(self, other: Self) -> Self {
            Self::new(self.x - other.x, self.y - other.y)
        }

        fn scale(self, factor: f64) -> Self {
            Self::new(self.x * factor, self.y * factor)
        }

        fn dot(self, other: Self) -> f64 {
            self.x.mul_add(other.x, self.y * other.y)
        }

        fn length_squared(self) -> f64 {
            self.dot(self)
        }

        fn normalized_or(self, fallback: Self) -> Self {
            let length = self.length_squared().sqrt();
            if length > f64::EPSILON {
                self.scale(length.recip())
            } else {
                fallback
            }
        }
    }

    #[derive(Clone, Copy, Debug, Default)]
    struct LinearColour {
        red: f64,
        green: f64,
        blue: f64,
    }

    impl LinearColour {
        const BLACK: Self = Self {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
        };

        fn add_scaled(&mut self, other: Self, scale: f64) {
            self.red = other.red.mul_add(scale, self.red).min(1.0);
            self.green = other.green.mul_add(scale, self.green).min(1.0);
            self.blue = other.blue.mul_add(scale, self.blue).min(1.0);
        }

        fn interpolate(self, other: Self, amount: f64) -> Self {
            Self {
                red: (other.red - self.red).mul_add(amount, self.red),
                green: (other.green - self.green).mul_add(amount, self.green),
                blue: (other.blue - self.blue).mul_add(amount, self.blue),
            }
        }

        fn rgb(self) -> Rgb {
            Rgb::new(channel(self.red), channel(self.green), channel(self.blue))
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the finite channel is rounded and clamped to the complete u8 range"
    )]
    fn channel(value: f64) -> u8 {
        value.mul_add(255.0, 0.0).round().clamp(0.0, 255.0) as u8
    }

    #[derive(Clone, Copy, Debug)]
    struct KeySample {
        name: &'static str,
        x: u8,
        y: u8,
    }

    impl KeySample {
        const fn new(name: &'static str, x: u8, y: u8) -> Self {
            Self { name, x, y }
        }

        fn position(self) -> Vector {
            Vector::new(
                f64::from(self.x).mul_add(0.5, 0.25),
                f64::from(self.y).mul_add(0.5, 0.5),
            )
        }
    }

    // These centres come from the photographed M16 R2 US-ANSI board. The
    // source coordinates occupy a 32-by-11 sampling plane; `position` maps
    // its six populated rows into the requested continuous 16-by-6 world.
    const KEY_SAMPLES: &[KeySample] = &[
        KeySample::new("escape", 0, 0),
        KeySample::new("f1", 3, 0),
        KeySample::new("f2", 5, 0),
        KeySample::new("f3", 7, 0),
        KeySample::new("f4", 9, 0),
        KeySample::new("f5", 12, 0),
        KeySample::new("f6", 14, 0),
        KeySample::new("f7", 16, 0),
        KeySample::new("f8", 18, 0),
        KeySample::new("f9", 21, 0),
        KeySample::new("f10", 23, 0),
        KeySample::new("f11", 25, 0),
        KeySample::new("f12", 27, 0),
        KeySample::new("home", 27, 0),
        KeySample::new("end", 29, 0),
        KeySample::new("delete", 31, 0),
        KeySample::new("mic-mute", 31, 2),
        KeySample::new("grave", 1, 2),
        KeySample::new("1", 3, 2),
        KeySample::new("2", 5, 2),
        KeySample::new("3", 7, 2),
        KeySample::new("4", 9, 2),
        KeySample::new("5", 11, 2),
        KeySample::new("6", 13, 2),
        KeySample::new("7", 15, 2),
        KeySample::new("8", 17, 2),
        KeySample::new("9", 19, 2),
        KeySample::new("0", 21, 2),
        KeySample::new("minus", 23, 2),
        KeySample::new("equals", 25, 2),
        KeySample::new("backspace", 28, 2),
        KeySample::new("volume-mute", 31, 4),
        KeySample::new("tab", 2, 4),
        KeySample::new("q", 4, 4),
        KeySample::new("w", 6, 4),
        KeySample::new("e", 8, 4),
        KeySample::new("r", 10, 4),
        KeySample::new("t", 12, 4),
        KeySample::new("y", 14, 4),
        KeySample::new("u", 16, 4),
        KeySample::new("i", 18, 4),
        KeySample::new("o", 20, 4),
        KeySample::new("p", 22, 4),
        KeySample::new("left-bracket", 24, 4),
        KeySample::new("right-bracket", 26, 4),
        KeySample::new("backslash", 28, 4),
        KeySample::new("volume-down", 31, 8),
        KeySample::new("volume-up", 31, 6),
        KeySample::new("caps-lock", 2, 6),
        KeySample::new("a", 5, 6),
        KeySample::new("s", 7, 6),
        KeySample::new("d", 9, 6),
        KeySample::new("f", 11, 6),
        KeySample::new("g", 13, 6),
        KeySample::new("h", 15, 6),
        KeySample::new("j", 17, 6),
        KeySample::new("k", 19, 6),
        KeySample::new("l", 21, 6),
        KeySample::new("semicolon", 23, 6),
        KeySample::new("apostrophe", 25, 6),
        KeySample::new("enter", 28, 6),
        KeySample::new("left-shift", 2, 8),
        KeySample::new("z", 6, 8),
        KeySample::new("x", 8, 8),
        KeySample::new("c", 10, 8),
        KeySample::new("v", 12, 8),
        KeySample::new("b", 14, 8),
        KeySample::new("n", 16, 8),
        KeySample::new("m", 18, 8),
        KeySample::new("comma", 20, 8),
        KeySample::new("period", 22, 8),
        KeySample::new("slash", 24, 8),
        KeySample::new("right-shift", 26, 8),
        KeySample::new("up", 29, 8),
        KeySample::new("left-ctrl", 1, 10),
        KeySample::new("fn", 4, 10),
        KeySample::new("left-win", 6, 10),
        KeySample::new("left-alt", 8, 10),
        KeySample::new("space", 13, 10),
        KeySample::new("right-win", 22, 10),
        KeySample::new("right-alt", 20, 10),
        KeySample::new("right-ctrl", 24, 10),
        KeySample::new("left", 27, 10),
        KeySample::new("down", 29, 10),
        KeySample::new("right", 31, 10),
    ];

    #[derive(Debug)]
    struct Options {
        balls: usize,
        input: PathBuf,
        seed: u64,
    }

    fn usage_error(message: impl Into<String>) -> Box<dyn Error> {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "{}\nusage: bouncy_balls --input /dev/input/...-event-kbd [--balls 1..=10] [--seed N]",
                message.into()
            ),
        )
        .into()
    }

    fn option_value(
        arguments: &mut impl Iterator<Item = OsString>,
        name: &str,
    ) -> DemoResult<OsString> {
        arguments
            .next()
            .ok_or_else(|| usage_error(format!("{name} requires a value")))
    }

    fn options_from(arguments: impl IntoIterator<Item = OsString>) -> DemoResult<Options> {
        let mut arguments = arguments.into_iter();
        let mut balls = DEFAULT_BALLS;
        let mut input = None;
        let mut seed = default_seed();

        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--balls") => {
                    let value = option_value(&mut arguments, "--balls")?;
                    balls = value
                        .to_str()
                        .ok_or_else(|| usage_error("--balls must be UTF-8"))?
                        .parse::<usize>()
                        .map_err(|_| usage_error("--balls must be an integer"))?;
                }
                Some("--input") => {
                    input = Some(PathBuf::from(option_value(&mut arguments, "--input")?));
                }
                Some("--seed") => {
                    let value = option_value(&mut arguments, "--seed")?;
                    seed = value
                        .to_str()
                        .ok_or_else(|| usage_error("--seed must be UTF-8"))?
                        .parse::<u64>()
                        .map_err(|_| usage_error("--seed must be an integer"))?;
                }
                _ => {
                    return Err(usage_error(format!(
                        "unknown argument: {}",
                        argument.to_string_lossy()
                    )));
                }
            }
        }
        if !(1..=MAX_BALLS).contains(&balls) {
            return Err(usage_error(format!(
                "--balls must be between 1 and {MAX_BALLS}"
            )));
        }
        let input = input.ok_or_else(|| usage_error("--input is required"))?;

        Ok(Options { balls, input, seed })
    }

    fn options() -> DemoResult<Options> {
        options_from(env::args_os().skip(1))
    }

    fn default_seed() -> u64 {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        elapsed.as_secs() ^ u64::from(elapsed.subsec_nanos())
    }

    #[derive(Debug)]
    struct Random(u64);

    impl Random {
        fn new(seed: u64) -> Self {
            Self(if seed == 0 {
                0x9e37_79b9_7f4a_7c15
            } else {
                seed
            })
        }

        fn next_u64(&mut self) -> u64 {
            let mut value = self.0;
            value ^= value << 13;
            value ^= value >> 7;
            value ^= value << 17;
            self.0 = value;
            value
        }

        fn unit(&mut self) -> f64 {
            let upper = u32::try_from(self.next_u64() >> 32).unwrap_or_default();
            f64::from(upper) / f64::from(u32::MAX)
        }

        fn direction(&mut self) -> Vector {
            let angle = self.unit() * TAU;
            Vector::new(angle.cos(), angle.sin())
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct Ball {
        position: Vector,
        velocity: Vector,
        colour: LinearColour,
    }

    #[derive(Debug)]
    struct World {
        balls: Vec<Ball>,
        pressed: BTreeSet<usize>,
        random: Random,
    }

    impl World {
        fn new(ball_count: usize, seed: u64) -> DemoResult<Self> {
            let mut random = Random::new(seed);
            let mut balls = Vec::with_capacity(ball_count);

            for index in 0..ball_count {
                let mut spawned = None;
                for _ in 0..MAX_SPAWN_ATTEMPTS {
                    let position = Vector::new(
                        BALL_RADIUS + (random.unit() * (WORLD_WIDTH - (2.0 * BALL_RADIUS))),
                        BALL_RADIUS + (random.unit() * (WORLD_HEIGHT - (2.0 * BALL_RADIUS))),
                    );
                    let clear = balls.iter().all(|ball: &Ball| {
                        ball.position.subtract(position).length_squared()
                            >= (2.1 * BALL_RADIUS).powi(2)
                    });
                    if clear {
                        spawned = Some(position);
                        break;
                    }
                }
                let position = spawned
                    .ok_or_else(|| io::Error::other("could not place all balls without overlap"))?;
                let hue = f64::from(u32::try_from(index).unwrap_or_default())
                    / f64::from(u32::try_from(ball_count).unwrap_or(1));
                balls.push(Ball {
                    position,
                    velocity: random.direction().scale(BALL_SPEED),
                    colour: rainbow(hue),
                });
            }

            Ok(Self {
                balls,
                pressed: BTreeSet::new(),
                random,
            })
        }

        fn set_key(&mut self, key_index: usize, key: KeySample, pressed: bool) {
            if !pressed {
                self.pressed.remove(&key_index);
                return;
            }
            if !self.pressed.insert(key_index) {
                return;
            }

            let centre = key.position();
            let reach_squared = (BALL_RADIUS + FINGER_RADIUS).powi(2);
            for ball in &mut self.balls {
                if ball.position.subtract(centre).length_squared() <= reach_squared {
                    ball.velocity = self.random.direction().scale(BALL_SPEED);
                }
            }
        }

        fn step(&mut self, keys: &[KeySample], seconds: f64) {
            for ball in &mut self.balls {
                ball.position = ball.position.add(ball.velocity.scale(seconds));
                collide_with_walls(ball);
            }

            let mut remaining = self.balls.as_mut_slice();
            while let Some((ball, others)) = remaining.split_first_mut() {
                for other in &mut *others {
                    collide_balls(ball, other);
                }
                remaining = others;
            }

            for &key_index in &self.pressed {
                let Some(key) = keys.get(key_index) else {
                    continue;
                };
                for ball in &mut self.balls {
                    collide_with_finger(ball, key.position());
                }
            }
        }
    }

    fn collide_with_walls(ball: &mut Ball) {
        if ball.position.x < BALL_RADIUS {
            ball.position.x = BALL_RADIUS;
            ball.velocity.x = ball.velocity.x.abs();
        } else if ball.position.x > WORLD_WIDTH - BALL_RADIUS {
            ball.position.x = WORLD_WIDTH - BALL_RADIUS;
            ball.velocity.x = -ball.velocity.x.abs();
        }
        if ball.position.y < BALL_RADIUS {
            ball.position.y = BALL_RADIUS;
            ball.velocity.y = ball.velocity.y.abs();
        } else if ball.position.y > WORLD_HEIGHT - BALL_RADIUS {
            ball.position.y = WORLD_HEIGHT - BALL_RADIUS;
            ball.velocity.y = -ball.velocity.y.abs();
        }
    }

    fn collide_balls(first: &mut Ball, second: &mut Ball) {
        let displacement = second.position.subtract(first.position);
        let minimum_distance = 2.0 * BALL_RADIUS;
        if displacement.length_squared() >= minimum_distance.powi(2) {
            return;
        }

        let normal = displacement.normalized_or(Vector::new(1.0, 0.0));
        let distance = displacement.length_squared().sqrt();
        let correction = normal.scale((minimum_distance - distance) / 2.0);
        first.position = first.position.subtract(correction);
        second.position = second.position.add(correction);

        let relative_speed = second.velocity.subtract(first.velocity).dot(normal);
        if relative_speed < 0.0 {
            first.velocity = first.velocity.add(normal.scale(relative_speed));
            second.velocity = second.velocity.subtract(normal.scale(relative_speed));
        }
    }

    fn collide_with_finger(ball: &mut Ball, centre: Vector) {
        let displacement = ball.position.subtract(centre);
        let minimum_distance = BALL_RADIUS + FINGER_RADIUS;
        if displacement.length_squared() >= minimum_distance.powi(2) {
            return;
        }

        let normal = displacement.normalized_or(
            ball.velocity
                .scale(-1.0)
                .normalized_or(Vector::new(1.0, 0.0)),
        );
        ball.position = centre.add(normal.scale(minimum_distance));
        let inward_speed = ball.velocity.dot(normal);
        if inward_speed < 0.0 {
            ball.velocity = ball.velocity.subtract(normal.scale(2.0 * inward_speed));
        }
    }

    fn rainbow(hue: f64) -> LinearColour {
        let phase = (hue.rem_euclid(1.0) * 6.0).clamp(0.0, 6.0);
        let crossing = 1.0 - ((phase % 2.0) - 1.0).abs();
        let (red, green, blue) = if phase < 1.0 {
            (1.0, crossing, 0.0)
        } else if phase < 2.0 {
            (crossing, 1.0, 0.0)
        } else if phase < 3.0 {
            (0.0, 1.0, crossing)
        } else if phase < 4.0 {
            (0.0, crossing, 1.0)
        } else if phase < 5.0 {
            (crossing, 0.0, 1.0)
        } else {
            (1.0, 0.0, crossing)
        };
        LinearColour { red, green, blue }
    }

    fn smooth_coverage(distance: f64, radius: f64) -> f64 {
        let feather = 0.18;
        let amount = ((radius + feather - distance) / (2.0 * feather)).clamp(0.0, 1.0);
        amount * amount * (3.0 - (2.0 * amount))
    }

    fn render_grid(world: &World, keys: &[KeySample]) -> Vec<LinearColour> {
        let mut grid = vec![LinearColour::BLACK; GRID_WIDTH * GRID_HEIGHT];
        let width = f64::from(u32::try_from(GRID_WIDTH).unwrap_or(1));
        let height = f64::from(u32::try_from(GRID_HEIGHT).unwrap_or(1));

        for (index, pixel) in grid.iter_mut().enumerate() {
            let x_index = index % GRID_WIDTH;
            let y_index = index / GRID_WIDTH;
            let point = Vector::new(
                (f64::from(u32::try_from(x_index).unwrap_or_default()) + 0.5) * WORLD_WIDTH / width,
                (f64::from(u32::try_from(y_index).unwrap_or_default()) + 0.5) * WORLD_HEIGHT
                    / height,
            );

            for ball in &world.balls {
                let distance = point.subtract(ball.position).length_squared().sqrt();
                pixel.add_scaled(ball.colour, smooth_coverage(distance, BALL_RADIUS));
            }
            for &key_index in &world.pressed {
                let Some(key) = keys.get(key_index) else {
                    continue;
                };
                let distance = point.subtract(key.position()).length_squared().sqrt();
                pixel.add_scaled(
                    LinearColour {
                        red: 0.08,
                        green: 0.18,
                        blue: 0.28,
                    },
                    smooth_coverage(distance, FINGER_RADIUS),
                );
            }
        }

        grid
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the coordinate is finite, non-negative, and clamped to the grid bounds"
    )]
    fn lower_grid_index(coordinate: f64, maximum: usize) -> usize {
        let maximum = f64::from(u32::try_from(maximum).unwrap_or(u32::MAX));
        coordinate.floor().clamp(0.0, maximum) as usize
    }

    fn grid_pixel(grid: &[LinearColour], x: usize, y: usize) -> LinearColour {
        grid.get((y * GRID_WIDTH) + x)
            .copied()
            .unwrap_or(LinearColour::BLACK)
    }

    fn sample_grid(grid: &[LinearColour], position: Vector) -> LinearColour {
        let grid_x = position.x.mul_add(
            f64::from(u32::try_from(GRID_WIDTH).unwrap_or(1)) / WORLD_WIDTH,
            -0.5,
        );
        let grid_y = position.y.mul_add(
            f64::from(u32::try_from(GRID_HEIGHT).unwrap_or(1)) / WORLD_HEIGHT,
            -0.5,
        );
        let x0 = lower_grid_index(grid_x, GRID_WIDTH - 1);
        let y0 = lower_grid_index(grid_y, GRID_HEIGHT - 1);
        let x1 = x0.saturating_add(1).min(GRID_WIDTH - 1);
        let y1 = y0.saturating_add(1).min(GRID_HEIGHT - 1);
        let x_amount = grid_x - grid_x.floor();
        let y_amount = grid_y - grid_y.floor();
        let top = grid_pixel(grid, x0, y0).interpolate(grid_pixel(grid, x1, y0), x_amount);
        let bottom = grid_pixel(grid, x0, y1).interpolate(grid_pixel(grid, x1, y1), x_amount);
        top.interpolate(bottom, y_amount)
    }

    fn render_frame(world: &World, keys: &[KeySample]) -> Vec<Colour> {
        let grid = render_grid(world, keys);
        keys.iter()
            .map(|key| Colour::rgb(sample_grid(&grid, key.position()).rgb()))
            .collect()
    }

    fn frame_keys(element_names: &[String]) -> DemoResult<Vec<KeySample>> {
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
                            format!("no M16 R2 simulation coordinate for key {name}"),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn key_name(code: KeyCode) -> Option<&'static str> {
        Some(match code {
            KeyCode::KEY_ESC => "escape",
            KeyCode::KEY_F1 => "f1",
            KeyCode::KEY_F2 => "f2",
            KeyCode::KEY_F3 => "f3",
            KeyCode::KEY_F4 => "f4",
            KeyCode::KEY_F5 => "f5",
            KeyCode::KEY_F6 => "f6",
            KeyCode::KEY_F7 => "f7",
            KeyCode::KEY_F8 => "f8",
            KeyCode::KEY_F9 => "f9",
            KeyCode::KEY_F10 => "f10",
            KeyCode::KEY_F11 => "f11",
            KeyCode::KEY_F12 => "f12",
            KeyCode::KEY_HOME => "home",
            KeyCode::KEY_END => "end",
            KeyCode::KEY_DELETE => "delete",
            KeyCode::KEY_MICMUTE => "mic-mute",
            KeyCode::KEY_GRAVE => "grave",
            KeyCode::KEY_1 => "1",
            KeyCode::KEY_2 => "2",
            KeyCode::KEY_3 => "3",
            KeyCode::KEY_4 => "4",
            KeyCode::KEY_5 => "5",
            KeyCode::KEY_6 => "6",
            KeyCode::KEY_7 => "7",
            KeyCode::KEY_8 => "8",
            KeyCode::KEY_9 => "9",
            KeyCode::KEY_0 => "0",
            KeyCode::KEY_MINUS => "minus",
            KeyCode::KEY_EQUAL => "equals",
            KeyCode::KEY_BACKSPACE => "backspace",
            KeyCode::KEY_MUTE => "volume-mute",
            KeyCode::KEY_TAB => "tab",
            KeyCode::KEY_Q => "q",
            KeyCode::KEY_W => "w",
            KeyCode::KEY_E => "e",
            KeyCode::KEY_R => "r",
            KeyCode::KEY_T => "t",
            KeyCode::KEY_Y => "y",
            KeyCode::KEY_U => "u",
            KeyCode::KEY_I => "i",
            KeyCode::KEY_O => "o",
            KeyCode::KEY_P => "p",
            KeyCode::KEY_LEFTBRACE => "left-bracket",
            KeyCode::KEY_RIGHTBRACE => "right-bracket",
            KeyCode::KEY_BACKSLASH => "backslash",
            KeyCode::KEY_VOLUMEDOWN => "volume-down",
            KeyCode::KEY_VOLUMEUP => "volume-up",
            KeyCode::KEY_CAPSLOCK => "caps-lock",
            KeyCode::KEY_A => "a",
            KeyCode::KEY_S => "s",
            KeyCode::KEY_D => "d",
            KeyCode::KEY_F => "f",
            KeyCode::KEY_G => "g",
            KeyCode::KEY_H => "h",
            KeyCode::KEY_J => "j",
            KeyCode::KEY_K => "k",
            KeyCode::KEY_L => "l",
            KeyCode::KEY_SEMICOLON => "semicolon",
            KeyCode::KEY_APOSTROPHE => "apostrophe",
            KeyCode::KEY_ENTER => "enter",
            KeyCode::KEY_LEFTSHIFT => "left-shift",
            KeyCode::KEY_Z => "z",
            KeyCode::KEY_X => "x",
            KeyCode::KEY_C => "c",
            KeyCode::KEY_V => "v",
            KeyCode::KEY_B => "b",
            KeyCode::KEY_N => "n",
            KeyCode::KEY_M => "m",
            KeyCode::KEY_COMMA => "comma",
            KeyCode::KEY_DOT => "period",
            KeyCode::KEY_SLASH => "slash",
            KeyCode::KEY_RIGHTSHIFT => "right-shift",
            KeyCode::KEY_UP => "up",
            KeyCode::KEY_LEFTCTRL => "left-ctrl",
            KeyCode::KEY_LEFTMETA => "left-win",
            KeyCode::KEY_LEFTALT => "left-alt",
            KeyCode::KEY_SPACE => "space",
            KeyCode::KEY_RIGHTMETA => "right-win",
            KeyCode::KEY_RIGHTALT => "right-alt",
            KeyCode::KEY_RIGHTCTRL => "right-ctrl",
            KeyCode::KEY_LEFT => "left",
            KeyCode::KEY_DOWN => "down",
            KeyCode::KEY_RIGHT => "right",
            _ => return None,
        })
    }

    struct GrabbedInput {
        events: EventStream,
        name: String,
        pressed: BTreeSet<KeyCode>,
    }

    impl GrabbedInput {
        fn open(path: &PathBuf) -> DemoResult<Self> {
            let mut device = Device::open(path).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("could not open input device {}: {error}", path.display()),
                )
            })?;
            let is_keyboard = device.supported_keys().is_some_and(|keys| {
                keys.contains(KeyCode::KEY_ESC)
                    && keys.contains(KeyCode::KEY_A)
                    && keys.contains(KeyCode::KEY_SPACE)
            });
            if !is_keyboard {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("{} is not a complete keyboard input device", path.display()),
                )
                .into());
            }
            let name = device.name().unwrap_or("unnamed keyboard").to_owned();
            device.grab().map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("could not exclusively grab {}: {error}", path.display()),
                )
            })?;
            let pressed = device.get_key_state()?.iter().collect();
            let events = device.into_event_stream()?;
            Ok(Self {
                events,
                name,
                pressed,
            })
        }

        async fn next_key(&mut self) -> io::Result<(KeyCode, i32)> {
            loop {
                let event = self.events.next_event().await?;
                if let EventSummary::Key(_, code, value) = event.destructure() {
                    update_pressed_keys(&mut self.pressed, code, value);
                    return Ok((code, value));
                }
            }
        }

        fn all_released(&self) -> io::Result<bool> {
            Ok(self
                .events
                .device()
                .get_key_state()?
                .iter()
                .next()
                .is_none())
        }
    }

    fn update_pressed_keys(pressed: &mut BTreeSet<KeyCode>, code: KeyCode, value: i32) {
        match value {
            0 => {
                pressed.remove(&code);
            }
            1 => {
                pressed.insert(code);
            }
            _ => {}
        }
    }

    impl Drop for GrabbedInput {
        fn drop(&mut self) {
            let _ = self.events.device_mut().ungrab();
        }
    }

    struct Fullscreen {
        output: io::Stdout,
    }

    struct ScreenStatus<'a> {
        input_name: &'a str,
        ball_count: usize,
        held_keys: usize,
        keys_down: usize,
        frames: u64,
        dropped: u64,
        exit_pending: bool,
    }

    impl Fullscreen {
        fn enter() -> DemoResult<Self> {
            let mut output = io::stdout();
            if !output.is_terminal() {
                return Err(
                    io::Error::other("fullscreen mode requires a terminal on stdout").into(),
                );
            }
            output.write_all(TERMINAL_ENTER)?;
            output.flush()?;
            Ok(Self { output })
        }

        fn draw(&mut self, status: &ScreenStatus<'_>) -> io::Result<()> {
            let instruction = if status.exit_pending {
                "Release all keys to leave safely."
            } else {
                "Press Escape to leave safely."
            };
            let ScreenStatus {
                input_name,
                ball_count,
                held_keys,
                keys_down,
                frames,
                dropped,
                ..
            } = status;
            write!(
                self.output,
                "\x1b[H\x1b[2J\x1b[1;36mLuminate bouncy balls\x1b[0m\n\n\
                 Keyboard: {input_name}\n\
                 Balls: {ball_count}/{MAX_BALLS}\n\
                 Held obstacles: {held_keys}\n\
                 Physical keys down: {keys_down}\n\
                 Frames: {frames} ({dropped} rate-limited)\n\n\
                 All keys on this input device are captured.\n\
                 Hold keys to obstruct and kick nearby balls.\n\
                 \x1b[1m{instruction}\x1b[0m\n"
            )?;
            self.output.flush()
        }
    }

    impl Drop for Fullscreen {
        fn drop(&mut self) {
            let _ = self.output.write_all(TERMINAL_LEAVE);
            let _ = self.output.flush();
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct PlaybackStats {
        frames: u64,
        dropped: u64,
    }

    fn update_world_key(
        world: &mut World,
        keys: &[KeySample],
        code: KeyCode,
        value: i32,
        exit_pending: bool,
    ) {
        if code == KeyCode::KEY_ESC {
            return;
        }
        let Some(name) = key_name(code) else {
            return;
        };
        let Some(key_index) = keys.iter().position(|key| key.name == name) else {
            return;
        };
        let Some(key) = keys.get(key_index).copied() else {
            return;
        };

        match value {
            0 => world.set_key(key_index, key, false),
            1 if !exit_pending => world.set_key(key_index, key, true),
            _ => {}
        }
    }

    async fn run_loop(
        client: &Client,
        target: &TargetId,
        generation: u32,
        keys: &[KeySample],
        world: &mut World,
        input: &mut GrabbedInput,
        screen: &mut Fullscreen,
    ) -> DemoResult<PlaybackStats> {
        let mut ticker = interval(Duration::from_nanos(1_000_000_000 / u64::from(FRAME_RATE)));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut stats = PlaybackStats {
            frames: 0,
            dropped: 0,
        };
        let mut exit_pending = false;

        loop {
            tokio::select! {
                key = input.next_key() => {
                    let (code, value) = key?;
                    if code == KeyCode::KEY_ESC && value == 1 {
                        exit_pending = true;
                    }
                    update_world_key(world, keys, code, value, exit_pending);
                    if exit_pending && input.all_released()? {
                        return Ok(stats);
                    }
                }
                _ = ticker.tick() => {
                    for _ in 0..PHYSICS_STEPS_PER_FRAME {
                        world.step(keys, f64::from(PHYSICS_RATE).recip());
                    }
                    let acknowledgement = client.upload_frame(
                        target.clone(),
                        FrameEnvelope {
                            generation,
                            sequence: stats.frames,
                            payload: FramePayload::Full(render_frame(world, keys)),
                            commit: false,
                        },
                    ).await?;
                    stats.dropped += u64::from(acknowledgement.dropped);
                    stats.frames = stats.frames.saturating_add(1);
                    screen.draw(&ScreenStatus {
                        input_name: &input.name,
                        ball_count: world.balls.len(),
                        held_keys: world.pressed.len(),
                        keys_down: input.pressed.len(),
                        frames: stats.frames,
                        dropped: stats.dropped,
                        exit_pending,
                    })?;
                }
            }
        }
    }

    async fn play(options: &Options) -> DemoResult<PlaybackStats> {
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
        let keys = frame_keys(&element_names)?;
        let mut world = World::new(options.balls, options.seed)?;
        let mut screen = Fullscreen::enter()?;
        let mut input = GrabbedInput::open(&options.input)?;
        let target = TargetId::surface(DEVICE_ID, SURFACE_ID);
        let generation = client.begin_frame_stream(target.clone()).await?;
        let result = run_loop(
            &client,
            &target,
            generation,
            &keys,
            &mut world,
            &mut input,
            &mut screen,
        )
        .await;
        let end_result = client.end_frame_stream(target, generation).await;

        result.and_then(|stats| end_result.map(|()| stats).map_err(Into::into))
    }

    pub async fn main() -> DemoResult<()> {
        let stats = play(&options()?).await?;
        println!(
            "played {} frames ({} rate-limited by the daemon)",
            stats.frames, stats.dropped
        );
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn test_ball(position: Vector, velocity: Vector) -> Ball {
            Ball {
                position,
                velocity,
                colour: LinearColour {
                    red: 1.0,
                    green: 0.0,
                    blue: 0.0,
                },
            }
        }

        #[test]
        fn options_enforce_the_ball_limit() {
            let error =
                options_from(["--input", "/dev/input/event0", "--balls", "11"].map(OsString::from))
                    .expect_err("eleven balls should be rejected");
            assert!(error.to_string().contains("between 1 and 10"), "{error}");
        }

        #[test]
        fn pressed_key_tracking_waits_for_releases_and_ignores_repeats() {
            let mut pressed = BTreeSet::new();

            update_pressed_keys(&mut pressed, KeyCode::KEY_A, 1);
            update_pressed_keys(&mut pressed, KeyCode::KEY_A, 2);
            update_pressed_keys(&mut pressed, KeyCode::KEY_ESC, 1);
            assert_eq!(pressed.len(), 2);

            update_pressed_keys(&mut pressed, KeyCode::KEY_ESC, 0);
            assert_eq!(
                pressed.iter().copied().collect::<Vec<_>>(),
                [KeyCode::KEY_A]
            );

            update_pressed_keys(&mut pressed, KeyCode::KEY_A, 0);
            assert!(pressed.is_empty());
        }

        #[test]
        fn wall_collision_reflects_velocity_without_changing_speed() {
            let mut ball = test_ball(Vector::new(0.1, 2.0), Vector::new(-3.0, 1.0));

            collide_with_walls(&mut ball);

            assert_eq!(ball.position.x, BALL_RADIUS);
            assert_eq!(ball.velocity, Vector::new(3.0, 1.0));
        }

        #[test]
        fn equal_mass_head_on_collision_exchanges_velocity() {
            let mut first = test_ball(Vector::new(4.0, 3.0), Vector::new(1.0, 0.0));
            let mut second = test_ball(Vector::new(4.8, 3.0), Vector::new(-1.0, 0.0));

            collide_balls(&mut first, &mut second);

            assert_eq!(first.velocity, Vector::new(-1.0, 0.0));
            assert_eq!(second.velocity, Vector::new(1.0, 0.0));
        }

        #[test]
        fn pressed_key_kicks_an_overlapping_ball_at_constant_speed() {
            let key = KeySample::new("a", 5, 6);
            let mut world = World {
                balls: vec![test_ball(key.position(), Vector::new(0.0, 0.0))],
                pressed: BTreeSet::new(),
                random: Random::new(7),
            };

            world.set_key(0, key, true);

            let speed = world
                .balls
                .first()
                .map_or(0.0, |ball| ball.velocity.length_squared().sqrt());
            assert!((speed - BALL_SPEED).abs() < 1e-9, "speed was {speed}");
            assert!(world.pressed.contains(&0));
        }

        #[test]
        fn rainbow_interpolates_distinct_primary_colours() {
            assert_eq!(rainbow(0.0).rgb(), Rgb::new(255, 0, 0));
            assert_eq!(rainbow(1.0 / 3.0).rgb(), Rgb::new(0, 255, 0));
            assert_eq!(rainbow(2.0 / 3.0).rgb(), Rgb::new(0, 0, 255));
        }

        #[test]
        fn subgrid_motion_changes_sampled_brightness_smoothly() {
            let key = KeySample::new("test", 15, 4);
            let position = key.position();
            let mut world = World {
                balls: vec![test_ball(
                    position.add(Vector::new(BALL_RADIUS, 0.0)),
                    Vector::default(),
                )],
                pressed: BTreeSet::new(),
                random: Random::new(1),
            };
            let first = sample_grid(&render_grid(&world, &[key]), position).red;
            if let Some(ball) = world.balls.first_mut() {
                ball.position.x += 0.04;
            }
            let second = sample_grid(&render_grid(&world, &[key]), position).red;

            assert!(first > second, "expected {first} to exceed {second}");
            assert!(second > 0.0, "the soft edge should remain partially lit");
            assert!(first - second < 0.5, "subgrid motion changed too abruptly");
        }

        #[test]
        fn layout_covers_all_published_keys() {
            assert_eq!(KEY_SAMPLES.len(), 85);
            let names = KEY_SAMPLES
                .iter()
                .map(|key| key.name)
                .collect::<BTreeSet<_>>();
            assert_eq!(names.len(), KEY_SAMPLES.len());
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> ExitCode {
    report(linux::main().await)
}
