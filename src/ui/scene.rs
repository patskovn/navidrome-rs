//! Looping pixel-art scenes, drawn in code, shown fullscreen behind the music.
//!
//! Two original scenes: a rainy night at a desk, for most music, and a
//! synthwave drive, for stations and albums that sound like one. Each is a
//! pure function of a frame number painting a small fixed canvas; the widget
//! scales that canvas up by whole pixels and draws it as half-blocks, the same
//! technique as the album covers (`cover.rs`), two real pixels per cell.
//!
//! **Why frames, not seconds.** The loop has to be seamless, forever. Every
//! moving part therefore runs a whole number of cycles per loop, and phase is
//! computed in integers — `frame * cycles mod FRAMES` — so frame `n` and frame
//! `n + FRAMES` are the same picture by construction rather than by floating
//! point luck. `a_scene_loops_exactly` holds every scene to that.

use std::f64::consts::TAU;

use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::widgets::Widget;

/// Canvas size in pixels: 16:9, small enough to read as pixel art.
pub const WIDTH: usize = 96;
pub const HEIGHT: usize = 54;

/// Frames per second the scene is designed for. The app redraws on its 50ms
/// tick, which is exactly this.
pub const FPS: u64 = 20;

/// Frames in one loop: eight seconds.
pub const FRAMES: u64 = 8 * FPS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    /// Night, rain on the window, a lamp, a mug, a sleeping cat.
    RainyDesk,
    /// Neon sun, mountains, a grid rolling towards you, palms.
    SynthwaveDrive,
}

impl Scene {
    pub const ALL: [Scene; 2] = [Scene::RainyDesk, Scene::SynthwaveDrive];

    /// The scene for what is playing: the drive for anything that names
    /// itself synthwave-ish, the desk for everything else.
    ///
    /// Read from the title, artist and album because a station is only a
    /// name and a URL on the server — its directory tags are not kept.
    pub fn for_track(track: Option<&crate::subsonic::Track>) -> Self {
        const DRIVE: &[&str] = &[
            "synth",
            "retro",
            "outrun",
            "vapor",
            "nightwave",
            "80s",
            "neon",
            "cyber",
        ];
        let Some(track) = track else {
            return Scene::RainyDesk;
        };
        let text = [
            Some(&track.title),
            track.artist.as_ref(),
            track.album.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(|s| s.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
        if DRIVE.iter().any(|word| text.contains(word)) {
            Scene::SynthwaveDrive
        } else {
            Scene::RainyDesk
        }
    }

    pub fn render(self, frame: u64) -> Canvas {
        match self {
            Scene::RainyDesk => rainy_desk(frame),
            Scene::SynthwaveDrive => synthwave_drive(frame),
        }
    }
}

/// The frame to show now. A process-wide clock, read at draw time the way the
/// visualiser reads its sample tap: animation is presentation, not state.
pub fn current_frame() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    (start.elapsed().as_millis() as u64) * FPS / 1000
}

/// Where in its cycle a part is: `cycles` whole turns per loop, `0.0..1.0`.
///
/// Integer arithmetic until the last step, which is what makes the loop exact.
fn phase(frame: u64, cycles: u64) -> f64 {
    ((frame % FRAMES) * cycles % FRAMES) as f64 / FRAMES as f64
}

/// `phase`, offset by a fixed fraction — for staggering identical parts.
fn phase_at(frame: u64, cycles: u64, offset: f64) -> f64 {
    (phase(frame, cycles) + offset).fract()
}

/// A small deterministic hash, for scattering rain, stars and lit windows
/// the same way every frame.
fn hash(n: u64) -> u64 {
    let mut x = n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^ (x >> 29)
}

fn unit(n: u64) -> f64 {
    (hash(n) % 10_000) as f64 / 10_000.0
}

type Rgb = [u8; 3];

const fn rgb(hex: u32) -> Rgb {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let channel = |i: usize| (a[i] as f64 + (b[i] as f64 - a[i] as f64) * t).round() as u8;
    [channel(0), channel(1), channel(2)]
}

/// One frame of a scene. Plain pixels; the widget does the scaling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canvas {
    pixels: Vec<Rgb>,
}

impl Canvas {
    fn new(fill: Rgb) -> Self {
        Self {
            pixels: vec![fill; WIDTH * HEIGHT],
        }
    }

    pub fn get(&self, x: usize, y: usize) -> Rgb {
        self.pixels[y.min(HEIGHT - 1) * WIDTH + x.min(WIDTH - 1)]
    }

    /// Writes a pixel; anything off the canvas is quietly dropped, which lets
    /// shapes run off the edge without every caller clipping.
    fn set(&mut self, x: i32, y: i32, color: Rgb) {
        if (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
            self.pixels[y as usize * WIDTH + x as usize] = color;
        }
    }

    fn blend(&mut self, x: i32, y: i32, color: Rgb, alpha: f64) {
        if (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
            let index = y as usize * WIDTH + x as usize;
            self.pixels[index] = mix(self.pixels[index], color, alpha);
        }
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Rgb) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, color);
            }
        }
    }

    /// Vertical gradient over rows `y0..y1`, through any number of stops.
    fn gradient(&mut self, x0: i32, x1: i32, y0: i32, y1: i32, stops: &[Rgb]) {
        let span = (y1 - y0 - 1).max(1) as f64;
        for y in y0..y1 {
            let t = (y - y0) as f64 / span * (stops.len() - 1) as f64;
            let i = (t.floor() as usize).min(stops.len() - 2);
            let color = mix(stops[i], stops[i + 1], t - i as f64);
            for x in x0..x1 {
                self.set(x, y, color);
            }
        }
    }

    fn disc(&mut self, cx: f64, cy: f64, r: f64, color: Rgb) {
        for y in (cy - r).floor() as i32..=(cy + r).ceil() as i32 {
            for x in (cx - r).floor() as i32..=(cx + r).ceil() as i32 {
                let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                if dx * dx + dy * dy <= r * r {
                    self.set(x, y, color);
                }
            }
        }
    }

    fn line(&mut self, (x0, y0): (f64, f64), (x1, y1): (f64, f64), color: Rgb) {
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            self.set(
                (x0 + (x1 - x0) * t).round() as i32,
                (y0 + (y1 - y0) * t).round() as i32,
                color,
            );
        }
    }
}

// ---------------------------------------------------------------- the desk --

fn rainy_desk(frame: u64) -> Canvas {
    // Wall, with faint wallpaper stripes.
    let mut c = Canvas::new(rgb(0x1c1733));
    c.gradient(0, WIDTH as i32, 0, 40, &[rgb(0x1a1530), rgb(0x2a1f45)]);
    for x in (3..WIDTH as i32).step_by(6) {
        for y in 0..40 {
            c.blend(x, y, rgb(0x3a2d5c), 0.25);
        }
    }

    // Window: frame, then night sky through the glass.
    let (gx0, gy0, gx1, gy1) = (12, 5, 58, 30);
    c.rect(
        gx0 - 2,
        gy0 - 2,
        gx1 - gx0 + 4,
        gy1 - gy0 + 4,
        rgb(0x4a3328),
    );
    c.gradient(
        gx0,
        gx1,
        gy0,
        gy1,
        &[rgb(0x0a1230), rgb(0x16224a), rgb(0x2a3566)],
    );
    c.disc(47.5, 10.5, 3.2, rgb(0xf3eac2));
    c.disc(46.5, 9.8, 2.6, rgb(0x0e173a)); // the crescent's shadow

    // City skyline, windows lit, a few going on and off through the loop.
    let buildings = [
        (12, 6, 9),
        (18, 5, 13),
        (23, 7, 7),
        (30, 4, 15),
        (34, 8, 10),
        (42, 5, 12),
        (47, 6, 6),
        (53, 5, 11),
    ];
    for (i, &(x, w, h)) in buildings.iter().enumerate() {
        let top = gy1 - h;
        c.rect(x, top, w, h, rgb(0x0a0e1f));
        for wy in (top + 2..gy1 - 1).step_by(2) {
            for wx in (x + 1..x + w - 1).step_by(2) {
                let seed = (i as u64) * 1000 + (wx as u64) * 31 + wy as u64;
                if !hash(seed).is_multiple_of(3) {
                    continue;
                }
                // Four steps per loop; a window flickers if its slot is "off".
                let step = (phase(frame, 4) * 4.0) as u64;
                let lit = !hash(seed + step).is_multiple_of(7);
                if lit {
                    c.set(wx, wy, rgb(0xffcf6b));
                }
            }
        }
    }

    // Rain running down the glass. Each drop falls a whole number of times
    // per loop, from its own starting height.
    for i in 0..40u64 {
        let x = gx0 + (hash(i) % (gx1 - gx0) as u64) as i32;
        let falls = 1 + hash(i + 100) % 3;
        let fall = phase_at(frame, falls, unit(i + 200));
        let y = gy0 as f64 + fall * (gy1 - gy0 + 2) as f64;
        for (k, alpha) in [(0, 0.75), (1, 0.45), (2, 0.2)] {
            let yy = y as i32 - k;
            if (gy0..gy1).contains(&yy) {
                c.blend(x, yy, rgb(0x9cc0f0), alpha);
            }
        }
    }

    // Mullions over the rain.
    c.rect(34, gy0, 2, gy1 - gy0, rgb(0x4a3328));
    c.rect(gx0, 17, gx1 - gx0, 1, rgb(0x4a3328));

    // Sill, and a plant on it that sways a little.
    c.rect(8, 31, 54, 3, rgb(0x5c4033));
    c.rect(8, 31, 54, 1, rgb(0x76523f));
    c.rect(15, 27, 6, 4, rgb(0xb5603c));
    c.rect(15, 27, 6, 1, rgb(0xc9734d));
    let sway = ((phase(frame, 1) * TAU).sin() * 1.2).round() as i32;
    for (dx, dy, color) in [
        (2, -1, 0x4f8a4b),
        (3, -1, 0x4f8a4b),
        (1, -2, 0x6fb35f),
        (4, -2, 0x6fb35f),
        (2, -3, 0x4f8a4b),
        (3, -4, 0x6fb35f),
        (0, -3, 0x4f8a4b),
        (5, -3, 0x4f8a4b),
    ] {
        let lean = if dy <= -3 { sway } else { 0 };
        c.set(15 + dx + lean, 27 + dy, rgb(color));
    }

    // Desk, with a little grain along its front.
    c.rect(0, 40, WIDTH as i32, 14, rgb(0x4a2f22));
    c.rect(0, 40, WIDTH as i32, 3, rgb(0x6b4632));
    c.rect(0, 40, WIDTH as i32, 1, rgb(0x80553d));
    for (y, x0, len) in [
        (45, 6, 18),
        (48, 30, 24),
        (51, 12, 14),
        (46, 60, 20),
        (50, 70, 16),
    ] {
        c.rect(x0, y, len, 1, rgb(0x553627));
    }

    // Lamp light first, so the lamp and everything under it sits in it: a
    // cone that is strongest along its axis and fades out to the sides, so it
    // reads as light rather than as a shape.
    let flicker = 1.0 + 0.05 * (phase(frame, 3) * TAU).sin();
    let warm = rgb(0xffc86e);
    for y in 27..40 {
        let half = 6.0 + (y - 27) as f64 * 0.8;
        for x in (79.0 - half).floor() as i32..=(79.0 + half).ceil() as i32 {
            let across = ((x as f64 - 79.0).abs() / half).min(1.0);
            c.blend(x, y, warm, 0.13 * (1.0 - across * across) * flicker);
        }
    }
    // Where it lands: a pool on the desktop.
    for y in 40..44 {
        for x in 58..WIDTH as i32 {
            let across = ((x as f64 - 79.0).abs() / 20.0).min(1.0);
            let down = (y - 40) as f64 / 4.0;
            c.blend(x, y, warm, 0.35 * (1.0 - across) * (1.0 - down) * flicker);
        }
    }

    // Books.
    c.rect(44, 37, 12, 3, rgb(0x7a3b45));
    c.rect(45, 34, 10, 3, rgb(0x3b5a7a));
    c.rect(44, 31, 11, 3, rgb(0xc79a3b));

    // Mug, and steam rising from it in three wisps.
    c.rect(61, 35, 6, 5, rgb(0xd8cfc0));
    c.rect(61, 35, 6, 1, rgb(0x5a3624));
    c.rect(67, 36, 1, 3, rgb(0xd8cfc0));
    c.set(68, 37, rgb(0xd8cfc0));
    for wisp in 0..3u64 {
        let x0 = 62.0 + wisp as f64 * 1.5;
        for puff in 0..6u64 {
            let rise = phase_at(frame, 2, wisp as f64 / 3.0 + puff as f64 / 6.0);
            let x = x0 + ((rise * 1.5 + wisp as f64 * 0.3) * TAU).sin() * 1.3;
            let y = 34.0 - rise * 11.0;
            c.blend(
                x.round() as i32,
                y.round() as i32,
                rgb(0xeee8de),
                (1.0 - rise) * 0.55,
            );
        }
    }

    // Lamp.
    c.rect(75, 38, 9, 2, rgb(0x2b2b33));
    c.rect(79, 27, 1, 11, rgb(0x2b2b33));
    for (i, y) in (21..27).enumerate() {
        let half = 2 + i as i32;
        c.rect(79 - half, y, half * 2 + 1, 1, rgb(0xe9b44c));
    }
    c.rect(73, 26, 13, 1, rgb(0xffe2a0));

    // The cat, asleep: body breathing, tail tip flicking.
    let breath = (phase(frame, 2) * TAU).sin();
    let fur = rgb(0x3a3542);
    let lit_fur = rgb(0x4b4458);
    let top = if breath > 0.4 { 33 } else { 34 };
    for y in top..40 {
        let inset = match y - top {
            0 => 3,
            1 => 1,
            _ => 0,
        };
        c.rect(24 + inset, y, 14 - inset * 2, 1, fur);
    }
    c.rect(26, top + 1, 9, 1, lit_fur);
    // Head, ears, closed eyes.
    c.rect(17, 34, 8, 6, fur);
    c.rect(18, 33, 6, 1, fur);
    for (x, y) in [(18, 32), (18, 31), (19, 32), (23, 32), (23, 31), (22, 32)] {
        c.set(x, y, fur);
    }
    c.set(18, 32, rgb(0x6a4f5f));
    c.set(23, 32, rgb(0x6a4f5f));
    c.rect(19, 36, 2, 1, rgb(0x8d82a3));
    c.rect(22, 36, 2, 1, rgb(0x8d82a3));
    c.set(21, 37, rgb(0xc98a9a));
    // Tail: lies along the desk, curls up at the end.
    let flick = (phase(frame, 2) * TAU).sin() * 0.5 + 0.5;
    for (i, (x, y)) in [(38, 39), (39, 39), (40, 39), (41, 39), (42, 38)]
        .into_iter()
        .enumerate()
    {
        c.set(x, y, if i % 2 == 0 { fur } else { lit_fur });
    }
    let tip = if flick > 0.66 {
        [(43, 37), (43, 36)]
    } else if flick > 0.33 {
        [(43, 37), (44, 37)]
    } else {
        [(43, 38), (44, 38)]
    };
    for (x, y) in tip {
        c.set(x, y, fur);
    }

    c
}

// --------------------------------------------------------------- the drive --

fn synthwave_drive(frame: u64) -> Canvas {
    const HORIZON: i32 = 32;
    let mut c = Canvas::new(rgb(0x12021f));

    c.gradient(
        0,
        WIDTH as i32,
        0,
        HORIZON,
        &[rgb(0x0e0220), rgb(0x2a0a52), rgb(0x7a1a7a), rgb(0xff3d7f)],
    );

    // Stars that twinkle, each at its own whole rate.
    for i in 0..45u64 {
        let x = (hash(i) % WIDTH as u64) as i32;
        let y = (hash(i + 500) % 18) as i32;
        let rate = 1 + hash(i + 900) % 3;
        let glow = (phase_at(frame, rate, unit(i + 1300)) * TAU).sin() * 0.5 + 0.5;
        c.blend(x, y, rgb(0xfff4ff), 0.25 + glow * 0.75);
    }

    // The sun: a gradient disc whose lower half is cut by bands that drift
    // down and thicken as they go.
    let (cx, cy, r) = (48.0, HORIZON as f64 - 3.0, 17.0);
    let drift = phase(frame, 2) * 4.0;
    for y in (cy - r) as i32..HORIZON {
        let t = (y as f64 - (cy - r)) / (2.0 * r);
        let color = mix(rgb(0xffe86b), rgb(0xff2f8f), t * 1.8);
        let below_middle = y as f64 - (cy - r * 0.55);
        if below_middle > 0.0 {
            let band = (below_middle + drift).rem_euclid(4.0);
            let gap = 0.4 + below_middle / r * 1.6;
            if band < gap {
                continue;
            }
        }
        for x in (cx - r) as i32..=(cx + r) as i32 {
            let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
            if dx * dx + dy * dy <= r * r {
                c.set(x, y, color);
            }
        }
    }

    // Two mountain ranges, the far one paler, each with a neon rim.
    // Peaks as (centre, height, half-width); the near range is darker and
    // lower, and leaves the middle open so the sun shows between them.
    let far: &[(f64, f64, f64)] = &[
        (14.0, 10.0, 14.0),
        (34.0, 7.0, 10.0),
        (62.0, 8.0, 11.0),
        (84.0, 11.0, 15.0),
    ];
    let near: &[(f64, f64, f64)] = &[
        (4.0, 7.0, 12.0),
        (24.0, 5.0, 9.0),
        (74.0, 6.0, 10.0),
        (93.0, 8.0, 11.0),
    ];
    for (peaks, seed, body, rim) in [
        (far, 3u64, rgb(0x3a1263), rgb(0xb04fd8)),
        (near, 7u64, rgb(0x1c0733), rgb(0xff4fd8)),
    ] {
        for x in 0..WIDTH as i32 {
            let h = peaks
                .iter()
                .map(|&(cx, h, hw)| h * (1.0 - (x as f64 - cx).abs() / hw))
                .fold(0.0_f64, f64::max);
            if h < 0.5 {
                continue;
            }
            // A pixel of roughness so the slopes are not ruler-straight.
            let jag = (hash(x as u64 + seed * 1000) % 3) as f64 * 0.5;
            let top = HORIZON - (h + jag).round() as i32;
            for y in top..HORIZON {
                c.set(x, y, body);
            }
            c.set(x, top, rim);
        }
    }

    // Ground, and the grid rolling towards the viewer.
    c.gradient(
        0,
        WIDTH as i32,
        HORIZON,
        HEIGHT as i32,
        &[rgb(0x1a0438), rgb(0x0b0220)],
    );
    let depth = (HEIGHT as i32 - HORIZON) as f64;
    let roll = phase(frame, 4);
    for k in 0..7 {
        let z = k as f64 + 1.0 - roll;
        if z <= 0.0 {
            continue;
        }
        let y = HORIZON as f64 + depth / z;
        // Lines this far out would only pile into a band at the horizon.
        if y < HORIZON as f64 + 2.5 {
            continue;
        }
        let fade = (1.0 / z).sqrt();
        for x in 0..WIDTH as i32 {
            c.blend(x, y.round() as i32, rgb(0xff2fd1), 0.2 + 0.8 * fade);
        }
    }
    // Lines out of the vanishing point, stepped along their longer axis so a
    // shallow one is a continuous stroke rather than a row of dots.
    for i in -8..=8 {
        let (x0, y0) = (48.0, HORIZON as f64);
        let (x1, y1) = (48.0 + i as f64 * 13.0, HEIGHT as f64);
        let steps = (x1 - x0).abs().max(y1 - y0).ceil() as i32;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
            if y < HORIZON as f64 + 2.0 {
                continue;
            }
            c.blend(
                x.round() as i32,
                y.round() as i32,
                rgb(0xff2fd1),
                0.15 + 0.85 * t,
            );
        }
    }
    c.rect(0, HORIZON, WIDTH as i32, 1, rgb(0xff6ad5));

    // Palms on either side, in silhouette against the sky.
    let silhouette = rgb(0x0c0016);
    for (root, lean) in [(9.0, 1.0), (86.0, -1.0)] {
        // The trunk bows out and back, as palms do.
        let top = (root + lean * 6.0, 22.0);
        let steps = 40;
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            let x = root + (top.0 - root) * t * t;
            let y = 53.0 + (top.1 - 53.0) * t;
            let width = if t < 0.6 { 2 } else { 1 };
            for w in 0..width {
                c.set(x.round() as i32 + w, y.round() as i32, silhouette);
            }
        }
        // Fronds arc out and droop, thick at the crown and thin at the tips;
        // they stir a little, once per loop.
        let stir = (phase(frame, 1) * TAU).sin() * 0.06;
        for (angle, length) in [
            (2.9, 13.0),
            (2.4, 11.0),
            (1.75, 8.0),
            (1.15, 11.0),
            (0.55, 13.0),
            (3.5, 9.0),
            (-0.35, 9.0),
        ] {
            let a: f64 = angle + stir;
            let steps = (length * 1.5) as i32;
            for i in 0..=steps {
                let s = i as f64 / steps as f64 * length;
                let x = top.0 + a.cos() * s;
                let y = top.1 - a.sin() * s + s * s * 0.045;
                c.set(x.round() as i32, y.round() as i32, silhouette);
                if s < length * 0.55 {
                    c.set(x.round() as i32, y.round() as i32 + 1, silhouette);
                }
            }
        }
    }

    c
}

// ------------------------------------------------------------------ widget --

/// Draws a [`Canvas`] as large as `area` allows at the canvas's proportions,
/// centred, with any margin left blank.
///
/// Scaling is nearest-neighbour by any factor, not only whole ones. Whole
/// steps kept every pixel the same size, but between 1× and 2× — which is
/// where a full-screen terminal usually lands — they left the picture at 1×,
/// a small island in the middle of the screen. A pixel that is now and then
/// one cell wider than its neighbour is the far smaller cost.
pub struct SceneWidget<'a> {
    canvas: &'a Canvas,
}

impl<'a> SceneWidget<'a> {
    pub fn new(canvas: &'a Canvas) -> Self {
        Self { canvas }
    }
}

impl Widget for SceneWidget<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Half-blocks: a cell is one pixel wide and two tall.
        let (avail_w, avail_h) = (area.width as usize, area.height as usize * 2);
        // As large as fits, keeping the canvas's proportions.
        let draw_w = avail_w.min(avail_h * WIDTH / HEIGHT).max(1);
        let draw_h = (draw_w * HEIGHT / WIDTH).clamp(1, avail_h);
        let rows = draw_h.div_ceil(2);
        let x0 = area.x as usize + (avail_w - draw_w) / 2;
        let y0 = area.y as usize + (area.height as usize - rows) / 2;

        let sample = |px: usize, py: usize| -> Option<Color> {
            (py < draw_h).then(|| {
                let [r, g, b] = self.canvas.get(px * WIDTH / draw_w, py * HEIGHT / draw_h);
                Color::Rgb(r, g, b)
            })
        };

        for row in 0..rows {
            for col in 0..draw_w {
                let Some(cell) = buf.cell_mut(((x0 + col) as u16, (y0 + row) as u16)) else {
                    continue;
                };
                let upper = sample(col, row * 2).unwrap_or(Color::Reset);
                let lower = sample(col, row * 2 + 1).unwrap_or(Color::Reset);
                cell.set_char('▀');
                cell.set_style(Style::new().fg(upper).bg(lower));
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use ratatui::buffer::Buffer;

    /// The seam: every scene, at any frame, is the same picture one loop on.
    #[test]
    fn a_scene_loops_exactly() {
        for scene in Scene::ALL {
            for frame in [0, 1, 37, 80, 159] {
                assert!(
                    scene.render(frame) == scene.render(frame + FRAMES),
                    "{scene:?} frame {frame} differs one loop later"
                );
            }
        }
    }

    /// …and it does move, all through the loop rather than in one corner.
    ///
    /// Sampled at frames that divide the loop unevenly: at exactly a half or
    /// a quarter, parts running 2 or 4 cycles per loop are back where they
    /// started by design, and the picture can look nearly still.
    #[test]
    fn a_scene_is_animated() {
        for scene in Scene::ALL {
            let a = scene.render(0);
            for later in [13, 57, 101] {
                let b = scene.render(later);
                let changed = (0..HEIGHT)
                    .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
                    .filter(|&(x, y)| a.get(x, y) != b.get(x, y))
                    .count();
                assert!(
                    changed > 40,
                    "{scene:?} barely moved by frame {later}: {changed} pixels"
                );
            }
        }
    }

    /// Consecutive frames differ only a little: motion, not flashing.
    #[test]
    fn consecutive_frames_change_gently() {
        for scene in Scene::ALL {
            for frame in [0, 50, FRAMES - 1] {
                let (a, b) = (scene.render(frame), scene.render(frame + 1));
                let changed = (0..HEIGHT)
                    .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
                    .filter(|&(x, y)| a.get(x, y) != b.get(x, y))
                    .count();
                assert!(
                    changed < WIDTH * HEIGHT / 4,
                    "{scene:?} frame {frame}→{} changed {changed} pixels",
                    frame + 1
                );
            }
        }
    }

    #[test]
    fn synthwave_names_get_the_drive() {
        let track = |title: &str| crate::subsonic::Track {
            title: title.into(),
            ..Default::default()
        };
        assert_eq!(
            Scene::for_track(Some(&track("Nightwave Plaza"))),
            Scene::SynthwaveDrive
        );
        assert_eq!(
            Scene::for_track(Some(&track("Esoterica — Synthwave"))),
            Scene::SynthwaveDrive
        );
        assert_eq!(
            Scene::for_track(Some(&track("Lofi 24/7"))),
            Scene::RainyDesk
        );
        assert_eq!(Scene::for_track(None), Scene::RainyDesk);
    }

    fn draw(width: u16, height: u16, canvas: &Canvas) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        SceneWidget::new(canvas).render(area, &mut buf);
        buf
    }

    /// Room for exactly twice the canvas: each source pixel becomes a crisp
    /// 2×2 block, so a cell's two halves are the same source pixel.
    #[test]
    fn whole_pixel_scaling_keeps_pixels_square_and_crisp() {
        let canvas = Scene::RainyDesk.render(0);
        let buf = draw((WIDTH * 2) as u16, HEIGHT as u16, &canvas);
        for (col, row) in [(0, 0), (37, 10), (WIDTH * 2 - 1, HEIGHT - 1)] {
            let cell = &buf[(col as u16, row as u16)];
            let [r, g, b] = canvas.get(col / 2, row);
            assert_eq!(cell.symbol(), "▀");
            assert_eq!(cell.fg, Color::Rgb(r, g, b), "at {col},{row}");
            assert_eq!(cell.bg, Color::Rgb(r, g, b), "at {col},{row}");
        }
    }

    /// Regression: scaling only by whole steps left a full-screen terminal —
    /// between 1× and 2× of the canvas — showing the scene at 1×, a small
    /// island. An in-between size now fills the width it has.
    #[test]
    fn in_between_sizes_fill_the_space() {
        let canvas = Scene::RainyDesk.render(0);
        // 150×120 pixels of room: the width runs out first, at 150×84 — that
        // is 42 rows, centred with 9 blank rows above and below.
        let buf = draw(150, 60, &canvas);
        assert_eq!(buf[(0, 9)].symbol(), "▀", "left edge used");
        assert_eq!(buf[(149, 9)].symbol(), "▀", "right edge used");
        assert_eq!(buf[(75, 8)].symbol(), " ", "blank above");
        assert_eq!(buf[(75, 51)].symbol(), " ", "blank below");
        assert_eq!(buf[(75, 50)].symbol(), "▀", "last row drawn");
    }

    /// A tall-enough terminal is filled to its height instead.
    #[test]
    fn a_wide_terminal_is_filled_to_its_height() {
        let canvas = Scene::SynthwaveDrive.render(0);
        // 200×108 pixels: the height runs out first, at 192×108.
        let buf = draw(200, 54, &canvas);
        assert_eq!(buf[(3, 0)].symbol(), " ");
        assert_eq!(buf[(4, 0)].symbol(), "▀", "top row drawn");
        assert_eq!(buf[(4, 53)].symbol(), "▀", "bottom row drawn");
        assert_eq!(buf[(196, 0)].symbol(), " ");
    }

    #[test]
    fn a_wide_area_centres_the_picture() {
        let canvas = Scene::SynthwaveDrive.render(0);
        let buf = draw(WIDTH as u16 + 20, (HEIGHT / 2) as u16, &canvas);
        assert_eq!(buf[(9, 0)].symbol(), " ", "left margin stays blank");
        assert_eq!(buf[(10, 0)].symbol(), "▀", "the picture starts after it");
    }

    #[test]
    fn tiny_and_empty_areas_do_not_panic() {
        let canvas = Scene::RainyDesk.render(0);
        for (w, h) in [(0, 0), (1, 1), (10, 3), (3, 40), (500, 2)] {
            draw(w, h, &canvas);
        }
    }

    /// Writes a few frames of each scene as PNGs, eight times enlarged:
    /// `NAVIDROME_RS_SCENE_DIR=/tmp/x cargo test preview_scenes -- --ignored`
    #[test]
    #[ignore = "writes image files for looking at"]
    fn preview_scenes() {
        let dir = std::env::var("NAVIDROME_RS_SCENE_DIR").expect("set NAVIDROME_RS_SCENE_DIR");
        for scene in Scene::ALL {
            for frame in [0, FRAMES / 3, FRAMES * 2 / 3] {
                let canvas = scene.render(frame);
                let zoom = 8;
                let image = image::RgbImage::from_fn(
                    (WIDTH * zoom) as u32,
                    (HEIGHT * zoom) as u32,
                    |x, y| image::Rgb(canvas.get(x as usize / zoom, y as usize / zoom)),
                );
                image
                    .save(format!("{dir}/{scene:?}-{frame:03}.png"))
                    .unwrap();
            }
        }
    }
}
