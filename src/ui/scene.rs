//! Looping pixel-art scenes, drawn in code, shown fullscreen behind the music.
//!
//! Two original scenes: a rainy night at a desk, for most music, and a
//! synthwave drive, for stations and albums that sound like one. Each is a
//! pure function of a frame number and a canvas size; the widget draws the
//! canvas as half-blocks, the same technique as the album covers
//! (`cover.rs`), two real pixels per cell.
//!
//! **Composed for the space, not stretched to it.** Every scene has a 96×54
//! *core* — the composition proper — and the widget picks the largest whole
//! pixel size at which that core fits. The canvas is then the whole area at
//! that pixel size, so pixels stay square and crisp and nothing is
//! letterboxed. The core sits bottom-centre, and whatever room is left is
//! furnished procedurally: the wall runs to the ceiling and out to both
//! sides, with shelves, lights, a clock and more desk on a wide terminal, or
//! more sky, peaks and palms on the drive.
//!
//! **Why frames, not seconds.** The loop has to be seamless, forever. Every
//! moving part therefore runs a whole number of cycles per loop, and phase is
//! computed in integers — `frame * cycles mod FRAMES` — so frame `n` and frame
//! `n + FRAMES` are the same picture by construction rather than by floating
//! point luck. `a_scene_loops_exactly` holds every scene to that, at several
//! sizes.

use std::f64::consts::TAU;

use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::widgets::Widget;

/// The core composition, in pixels: 16:9, small enough to read as pixel art.
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
    /// Neon sun, mountains, a grid rolling towards you, palms, a car.
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

    /// One frame on a `width`×`height` canvas. The core composition keeps its
    /// size and sits bottom-centre; the rest is furnished to fit.
    pub fn render(self, frame: u64, width: usize, height: usize) -> Canvas {
        let mut canvas = Canvas::new(width.max(1), height.max(1));
        match self {
            Scene::RainyDesk => rainy_desk(&mut canvas, frame),
            Scene::SynthwaveDrive => synthwave_drive(&mut canvas, frame),
        }
        canvas
    }
}

/// The frame to show now. A process-wide clock, read at draw time the way the
/// visualiser reads its sample tap: animation is presentation, not state.
pub fn current_frame() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    (start.elapsed().as_millis() as u64) * FPS / 1000
}

/// How a scene fits an area of `cols`×`rows` cells: the pixel size, and the
/// canvas in pixels at that size.
///
/// The pixel size is the largest whole number at which the core still fits,
/// but never below one: a terminal smaller than the core gets a canvas
/// smaller than the core, cropped from the top and sides.
pub fn fit(cols: u16, rows: u16) -> (usize, usize, usize) {
    // Half-blocks: a cell is one pixel wide and two tall.
    let (w, h) = (cols as usize, rows as usize * 2);
    let scale = (w / WIDTH).min(h / HEIGHT).max(1);
    (scale, w / scale, h / scale)
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

/// `0.0..=1.0`, rising and falling `cycles` times per loop.
fn wave(frame: u64, cycles: u64, offset: f64) -> f64 {
    (phase_at(frame, cycles, offset) * TAU).sin() * 0.5 + 0.5
}

/// A small deterministic hash, for scattering rain, stars, books and lit
/// windows the same way every frame.
fn hash(n: u64) -> u64 {
    let mut x = n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^ (x >> 29)
}

fn unit(n: u64) -> f64 {
    (hash(n) % 10_000) as f64 / 10_000.0
}

/// A hash of a signed position, for things laid out across a canvas whose
/// left edge may be at a negative core coordinate.
fn hash_at(x: i32, salt: u64) -> u64 {
    hash((x as i64 as u64).wrapping_add(salt.wrapping_mul(0x1000_0000)))
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

/// One frame of a scene.
///
/// Drawing goes through an *origin*: the scenes place their core with
/// coordinates relative to it, and move it to `(0, 0)` for anything laid out
/// across the whole canvas. Anything off the canvas is clipped, so shapes may
/// run over any edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<Rgb>,
    origin: (i32, i32),
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![[0, 0, 0]; width * height],
            origin: (0, 0),
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// The pixel at absolute `(x, y)`, clamped to the canvas.
    pub fn get(&self, x: usize, y: usize) -> Rgb {
        self.pixels[y.min(self.height - 1) * self.width + x.min(self.width - 1)]
    }

    fn origin(&mut self, x: i32, y: i32) {
        self.origin = (x, y);
    }

    /// Absolute index of a point given relative to the origin, if on canvas.
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (x + self.origin.0, y + self.origin.1);
        ((0..self.width as i32).contains(&x) && (0..self.height as i32).contains(&y))
            .then(|| y as usize * self.width + x as usize)
    }

    fn set(&mut self, x: i32, y: i32, color: Rgb) {
        if let Some(i) = self.index(x, y) {
            self.pixels[i] = color;
        }
    }

    fn blend(&mut self, x: i32, y: i32, color: Rgb, alpha: f64) {
        if let Some(i) = self.index(x, y) {
            self.pixels[i] = mix(self.pixels[i], color, alpha);
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

    /// A soft round glow, strongest at the centre.
    fn glow(&mut self, cx: f64, cy: f64, r: f64, color: Rgb, strength: f64) {
        for y in (cy - r).floor() as i32..=(cy + r).ceil() as i32 {
            for x in (cx - r).floor() as i32..=(cx + r).ceil() as i32 {
                let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                let d = (dx * dx + dy * dy).sqrt() / r;
                if d < 1.0 {
                    self.blend(x, y, color, strength * (1.0 - d) * (1.0 - d));
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

/// Where the core goes on a canvas: centred across, flush with the bottom.
/// Negative on a canvas smaller than the core, which crops it evenly.
fn core_origin(c: &Canvas) -> (i32, i32) {
    (
        (c.width as i32 - WIDTH as i32) / 2,
        c.height as i32 - HEIGHT as i32,
    )
}

// ---------------------------------------------------------------- the desk --

/// The desk scene's own anchor: like [`core_origin`], but raised until the
/// cat — whose lowest row is the core's row 39 — sits at least a third of the
/// height up from the bottom. The desk below simply grows.
///
/// Never more than four rows above the canvas top, which on the smallest
/// canvas costs one row of the window frame and nothing else.
fn desk_origin(c: &Canvas) -> (i32, i32) {
    let (ox, bottom_anchored) = core_origin(c);
    let h = c.height as i32;
    // The cat's bottom row is `oy + 39`; below it must be at least `h / 3`.
    let third_up = h - (h + 2) / 3 - 40;
    (ox, bottom_anchored.min(third_up).max(-4))
}

const WALL_TOP: Rgb = rgb(0x120e24);
const WALL: Rgb = rgb(0x1a1530);
const WALL_LOW: Rgb = rgb(0x2a1f45);
const WOOD_DARK: Rgb = rgb(0x4a2f22);
const WOOD: Rgb = rgb(0x6b4632);
const WOOD_EDGE: Rgb = rgb(0x80553d);
const FRAME_WOOD: Rgb = rgb(0x4a3328);
const WARM: Rgb = rgb(0xffc86e);
const LEAF: Rgb = rgb(0x4f8a4b);
const LEAF_LIT: Rgb = rgb(0x6fb35f);
const POT: Rgb = rgb(0xb5603c);

fn rainy_desk(c: &mut Canvas, frame: u64) {
    let (ox, oy) = desk_origin(c);
    let (w, h) = (c.width as i32, c.height as i32);
    let desk = oy + 40; // the desktop's top edge, in absolute rows
    let left = ox; // room beside the core, in pixels
    let right = w - (ox + WIDTH as i32);

    // --- The room, across the whole canvas. ---
    c.origin(0, 0);
    c.gradient(0, w, 0, desk, &[WALL_TOP, WALL, WALL_LOW]);
    // Wallpaper stripes, aligned to the core so they do not shift with size.
    for x in 0..w {
        if (x - ox - 3).rem_euclid(6) == 0 {
            for y in 0..desk {
                c.blend(x, y, rgb(0x3a2d5c), 0.22);
            }
        }
    }
    // A ceiling moulding, when there is ceiling to see.
    if oy >= 4 {
        c.rect(0, 0, w, 2, rgb(0x0d0a1a));
        c.rect(0, 2, w, 1, rgb(0x2c2240));
    }

    // Left of the core: shelves of books on the wall, a trailing plant, and on
    // the desk a record player, then cacti as far as there is desk.
    if left >= 12 {
        wall_shelves(c, frame, 2, left - 3, oy);
    }
    // Right of the core: a clock and a poster on the wall, a laptop on the
    // desk, then plants.
    let rx = ox + WIDTH as i32; // first column right of the core
    if right >= 24 {
        clock(c, frame, rx + 11, oy + 11);
    }
    if right >= 44 {
        poster(c, rx + 25, oy + 5);
    }
    if right >= 66 {
        poster_small(c, rx + 48, oy + 9);
    }

    // Pictures above the shelves, when the wall is tall enough to hang them
    // clear of the fairy lights.
    if left >= 26 && oy >= 28 {
        poster_small(c, ox - 22, oy - 24);
    }
    if left >= 46 && oy >= 30 {
        poster(c, ox - 44, oy - 28);
    }
    // A macramé plant hanging from the ceiling, left of the window.
    if oy >= 16 {
        hanging_plant(c, frame, ox + 3, 3, oy - 9);
    }

    // Fairy lights strung across the top of the wall.
    if oy >= 6 {
        fairy_lights(c, frame, 0, w, oy - 5);
    }

    // --- The core. ---
    c.origin(ox, oy);
    window(c, frame);

    // A gallery over the window on a tall wall.
    if oy >= 34 {
        poster(c, 26, -30);
    }
    if oy >= 28 {
        leaf_print(c, 46, -24);
    }
    // The clock goes above the lamp when there is no room for it beside.
    if right < 24 && oy >= 22 {
        c.origin(0, 0);
        clock(c, frame, ox + 84, oy - 13);
        c.origin(ox, oy);
    }
    // A pendant lamp between the window and the desk lamp, when the ceiling
    // is high enough to hang one.
    if oy >= 12 {
        pendant(c, frame, 66, -oy + 3, -2);
    }

    // The desk spans the canvas: its top, a front apron, and below that the
    // room carries on under it.
    c.origin(0, 0);
    c.rect(0, desk, w, h - desk, WOOD_DARK);
    c.rect(0, desk, w, 3, WOOD);
    c.rect(0, desk, w, 1, WOOD_EDGE);
    under_desk(c, ox, desk, w, h);

    // Desk things beside the core.
    if left >= 28 {
        record_player(c, frame, ox - 27, desk);
    }
    let mut x = ox - 40;
    while x > 2 {
        cactus(c, frame, x, desk, hash_at(x - ox, 11));
        x -= 15;
    }
    if right >= 24 {
        laptop(c, frame, rx + 3, desk);
    }
    let mut x = rx + 26;
    while x < w - 6 {
        cactus(c, frame, x, desk, hash_at(x - ox, 13));
        x += 15;
    }

    // --- The core's desk: light, books, mug, lamp, cat. ---
    c.origin(ox, oy);
    desk_core(c, frame);
}

/// Rows of desk front between the desktop and the space beneath it.
const APRON: i32 = 4;

/// Below the desktop: the apron with a drawer, legs, the back wall in shadow,
/// a floor with a rug, and a basket of yarn on it.
///
/// Laid out from whatever height is left. With too little, the space is
/// all apron and wood, as the desk used to be; items that would not fit are
/// left out rather than squashed.
fn under_desk(c: &mut Canvas, ox: i32, desk: i32, w: i32, h: i32) {
    let apron_top = desk + 3;
    let space_top = apron_top + APRON;
    let room = h - space_top;
    // A few rows only: leave it as plain desk front.
    if room < 6 {
        return;
    }

    // Apron: a grain line, and a drawer under the left of the core.
    let mut x = 0;
    while x < w {
        let len = 8 + (hash_at(x - ox, 1) % 18) as i32;
        if !hash_at(x - ox, 8).is_multiple_of(3) {
            c.rect(x, apron_top + 2, len, 1, rgb(0x553627));
        }
        x += len + 4 + (hash_at(x - ox, 4) % 10) as i32;
    }
    c.rect(ox + 8, apron_top, 26, APRON, rgb(0x56382a));
    c.rect(ox + 8, apron_top, 26, 1, rgb(0x6b4632));
    c.rect(ox + 20, apron_top + 1, 3, 2, rgb(0xd9a441));
    c.set(ox + 20, apron_top + 1, rgb(0xf3cf6e));
    c.rect(0, space_top - 1, w, 1, rgb(0x2e1d15));

    // The floor takes the bottom two fifths of the space, at least five rows.
    let floor = h - (room * 2 / 5).max(5);
    let skirting = floor - 2;

    // The back wall, in the desk's shadow: darkest just under the desktop.
    c.gradient(0, w, space_top, skirting, &[rgb(0x0e0a1a), rgb(0x1d1630)]);
    for x in 0..w {
        if (x - ox - 3).rem_euclid(6) == 0 {
            for y in space_top..skirting {
                c.blend(x, y, rgb(0x3a2d5c), 0.12);
            }
        }
    }
    c.rect(0, skirting, w, 2, rgb(0x2a2035));
    c.rect(0, skirting, w, 1, rgb(0x3a2d48));

    // Floorboards: planks three rows deep, their joints staggered.
    for y in floor..h {
        let plank = (y - floor) / 3;
        let base = if plank % 2 == 0 {
            rgb(0x4a3127)
        } else {
            rgb(0x533629)
        };
        for x in 0..w {
            c.set(x, y, base);
        }
        if (y - floor) % 3 == 0 {
            c.rect(0, y, w, 1, rgb(0x3a261e));
        }
        let offset = (hash(plank as u64 + 77) % 23) as i32;
        let mut jx = offset - ox.rem_euclid(23);
        while jx < w {
            c.set(jx, y, rgb(0x3a261e));
            jx += 23 + (hash_at(jx, plank as u64 * 31) % 9) as i32;
        }
    }

    // The rug, under the core: a border, a diamond band, fringe at the ends.
    let rug_top = floor + 1;
    let rug_bottom = h - 1;
    if rug_bottom - rug_top >= 2 {
        let (rx0, rx1) = (ox + 10, ox + 86);
        c.rect(rx0, rug_top, rx1 - rx0, rug_bottom - rug_top, rgb(0x7a2e43));
        c.rect(rx0, rug_top, rx1 - rx0, 1, rgb(0xc46a80));
        c.rect(rx0, rug_bottom - 1, rx1 - rx0, 1, rgb(0xc46a80));
        let mid = (rug_top + rug_bottom) / 2;
        for x in rx0 + 2..rx1 - 2 {
            let d = (x - rx0).rem_euclid(8) - 4;
            for y in rug_top + 1..rug_bottom - 1 {
                if (y - mid).abs() == d.abs() % 4 {
                    c.set(x, y, rgb(0xd99a5b));
                }
            }
        }
        for y in rug_top..rug_bottom {
            if (y - rug_top) % 2 == 0 {
                c.set(rx0 - 1, y, rgb(0xe8d8c0));
                c.set(rx1, y, rgb(0xe8d8c0));
            }
        }
    }

    // Legs, at the core's corners and at the canvas's ends.
    let leg = rgb(0x3a2419);
    let mut legs = vec![ox + 1, ox + 92];
    if ox >= 8 {
        legs.extend([1, w - 4]);
    }
    for x in legs {
        c.rect(x, space_top, 3, skirting + 1 - space_top, leg);
        c.rect(x, space_top, 1, skirting + 1 - space_top, rgb(0x4a2f22));
    }

    // A basket of yarn: woven, three balls in it, knitting needles stuck in.
    if floor - space_top >= 10 {
        let (bx, by) = (ox + 56, floor - 4);
        // Needles first, behind the balls, crossed and leaning out.
        for (x0, x1) in [(bx + 3, bx - 2), (bx + 9, bx + 14)] {
            c.line(
                (x0 as f64, (by + 1) as f64),
                (x1 as f64, (by - 8) as f64),
                rgb(0xcfc6b8),
            );
        }
        c.set(bx - 2, by - 9, rgb(0xd1495b));
        c.set(bx + 14, by - 9, rgb(0x7ee0d6));
        // The balls, each with a few turns of winding and a highlight.
        for (dx, dy, color) in [(3, -2, 0xd1495b), (11, -2, 0x7ee0d6), (7, -4, 0xe9c46a)] {
            let (cx, cy) = ((bx + dx) as f64 + 0.5, (by + dy) as f64 + 0.5);
            let color = rgb(color);
            c.disc(cx, cy, 3.2, color);
            let shade = mix(color, rgb(0x000000), 0.35);
            for k in -1..=1 {
                c.line(
                    (cx - 2.0, cy + k as f64 * 1.5 + 1.0),
                    (cx + 2.0, cy + k as f64 * 1.5 - 1.0),
                    shade,
                );
            }
            c.set(cx as i32 - 1, cy as i32 - 2, mix(color, rgb(0xffffff), 0.5));
        }
        // The basket: a rim, and weave in alternating rows.
        c.rect(bx, by, 15, 5, rgb(0x9c7648));
        for y in by + 1..by + 5 {
            for x in bx..bx + 15 {
                if (x + y).rem_euclid(3) == 0 {
                    c.set(x, y, rgb(0x7a5a34));
                }
            }
        }
        c.rect(bx - 1, by, 17, 1, rgb(0xc9a46e));
        c.rect(bx, by + 4, 15, 1, rgb(0x6b4e2c));
    }
}

fn window(c: &mut Canvas, frame: u64) {
    let (gx0, gy0, gx1, gy1) = (12, 5, 58, 30);
    c.rect(gx0 - 2, gy0 - 2, gx1 - gx0 + 4, gy1 - gy0 + 4, FRAME_WOOD);
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
                if !hash(seed + step).is_multiple_of(7) {
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
    c.rect(34, gy0, 2, gy1 - gy0, FRAME_WOOD);
    c.rect(gx0, 17, gx1 - gx0, 1, FRAME_WOOD);

    // Sill, and a plant on it that sways a little.
    c.rect(8, 31, 54, 3, rgb(0x5c4033));
    c.rect(8, 31, 54, 1, rgb(0x76523f));
    c.rect(15, 27, 6, 4, POT);
    c.rect(15, 27, 6, 1, rgb(0xc9734d));
    let sway = ((phase(frame, 1) * TAU).sin() * 1.2).round() as i32;
    for (dx, dy, color) in [
        (2, -1, LEAF),
        (3, -1, LEAF),
        (1, -2, LEAF_LIT),
        (4, -2, LEAF_LIT),
        (2, -3, LEAF),
        (3, -4, LEAF_LIT),
        (0, -3, LEAF),
        (5, -3, LEAF),
    ] {
        let lean = if dy <= -3 { sway } else { 0 };
        c.set(15 + dx + lean, 27 + dy, color);
    }
}

fn desk_core(c: &mut Canvas, frame: u64) {
    // Lamp light first, so the lamp and everything under it sits in it: a
    // cone that is strongest along its axis and fades out to the sides, so it
    // reads as light rather than as a shape.
    let flicker = 1.0 + 0.05 * (phase(frame, 3) * TAU).sin();
    for y in 27..40 {
        let half = 6.0 + (y - 27) as f64 * 0.8;
        for x in (79.0 - half).floor() as i32..=(79.0 + half).ceil() as i32 {
            let across = ((x as f64 - 79.0).abs() / half).min(1.0);
            c.blend(x, y, WARM, 0.13 * (1.0 - across * across) * flicker);
        }
    }
    // Where it lands: a pool on the desktop.
    for y in 40..44 {
        for x in 58..WIDTH as i32 + 4 {
            let across = ((x as f64 - 79.0).abs() / 20.0).min(1.0);
            let down = (y - 40) as f64 / 4.0;
            c.blend(x, y, WARM, 0.35 * (1.0 - across) * (1.0 - down) * flicker);
        }
    }

    // A cup of pens in the corner.
    c.rect(4, 35, 5, 5, rgb(0x3b5a7a));
    c.rect(4, 35, 5, 1, rgb(0x5378a0));
    for (x, top, color) in [(5, 31, 0xe9c46a), (6, 32, 0xd1495b), (7, 30, 0xeeeeee)] {
        c.rect(x, top, 1, 35 - top, rgb(color));
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

    // A small succulent at the far corner.
    c.rect(88, 37, 5, 3, rgb(0xd8cfc0));
    for (dx, dy) in [(1, -1), (2, -2), (3, -1), (0, -2), (4, -2), (2, -3)] {
        c.set(88 + dx, 37 + dy, if dy == -3 { LEAF_LIT } else { LEAF });
    }

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
    let flick = wave(frame, 2, 0.0);
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
}

/// Two shelves on the wall between absolute columns `x0..x1`, filled with
/// books, with a pothos trailing off the top one.
fn wall_shelves(c: &mut Canvas, frame: u64, x0: i32, x1: i32, oy: i32) {
    let palette = [
        0x7a3b45, 0x3b5a7a, 0xc79a3b, 0x4f6b4a, 0x8a5a9e, 0xb5603c, 0x2f4858, 0xd8cfc0,
    ];
    for (shelf, y) in [(0u64, oy + 13), (1, oy + 24)] {
        c.rect(x0, y, x1 - x0, 2, WOOD);
        c.rect(x0, y, x1 - x0, 1, WOOD_EDGE);
        c.rect(x0 + 2, y + 2, 1, 2, WOOD_DARK);
        c.rect(x1 - 3, y + 2, 1, 2, WOOD_DARK);
        // Books stand on the shelf, leaning now and then; gaps here and there.
        let mut x = x0 + 1;
        while x < x1 - 2 {
            let seed = hash(x as u64 * 131 + shelf * 7);
            let width = 1 + (seed % 3) as i32;
            if seed.is_multiple_of(11) {
                x += 3;
                continue;
            }
            let height = 5 + (seed / 7 % 4) as i32;
            let color = rgb(palette[(seed / 13 % palette.len() as u64) as usize]);
            let width = width.min(x1 - 2 - x);
            c.rect(x, y - height, width, height, color);
            c.rect(x, y - height + 1, width, 1, mix(color, rgb(0xffffff), 0.25));
            x += width;
        }
    }
    // The pothos: vines down from the top shelf's end, swaying a little.
    let px = x1 - 7;
    c.rect(px, oy + 9, 5, 4, POT);
    for vine in 0..3 {
        let length = 6 + vine * 4;
        for i in 0..length {
            let sway = (phase_at(frame, 1, vine as f64 * 0.2) * TAU).sin()
                * (i as f64 / length as f64)
                * 1.5;
            let x = px + 1 + vine + sway.round() as i32;
            let y = oy + 15 + i;
            c.set(x, y, if i % 3 == 0 { LEAF_LIT } else { LEAF });
        }
    }
}

/// A macramé hanger from the ceiling at absolute `x`, rows `top..bottom`,
/// holding a pot whose vines spill over and sway.
fn hanging_plant(c: &mut Canvas, frame: u64, x: i32, top: i32, bottom: i32) {
    let cord = rgb(0xc9b28a);
    let sway = (phase_at(frame, 1, 0.3) * TAU).sin() * 0.7;
    let px = x as f64 + sway;
    // Three cords meeting at a knot, splaying to the pot's rim.
    c.line((x as f64, top as f64), (px, (bottom - 6) as f64), cord);
    c.line(
        (px, (bottom - 6) as f64),
        (px - 3.0, (bottom - 1) as f64),
        cord,
    );
    c.line(
        (px, (bottom - 6) as f64),
        (px + 3.0, (bottom - 1) as f64),
        cord,
    );
    let pot_x = px.round() as i32 - 3;
    c.rect(pot_x, bottom - 1, 7, 4, rgb(0xd8cfc0));
    c.rect(pot_x, bottom - 1, 7, 1, rgb(0xeee8de));
    // Leaves over the rim, and vines trailing down.
    for (dx, dy) in [
        (1, -2),
        (2, -3),
        (4, -2),
        (5, -3),
        (3, -4),
        (0, -1),
        (6, -1),
    ] {
        c.set(
            pot_x + dx,
            bottom - 1 + dy,
            if dy == -4 { LEAF_LIT } else { LEAF },
        );
    }
    for (vine, length) in [(0, 7), (6, 10), (2, 4)] {
        for i in 0..length {
            let swing = (phase_at(frame, 1, vine as f64 * 0.1) * TAU).sin()
                * (i as f64 / length as f64)
                * 1.2;
            c.set(
                pot_x + vine + swing.round() as i32,
                bottom + 3 + i,
                if i % 3 == 1 { LEAF_LIT } else { LEAF },
            );
        }
    }
}

/// A wall clock centred at `(cx, cy)`, absolute. The second hand ticks round
/// once per loop, in eight steps.
fn clock(c: &mut Canvas, frame: u64, cx: i32, cy: i32) {
    let (x, y) = (cx as f64 + 0.5, cy as f64 + 0.5);
    c.disc(x, y, 7.0, rgb(0x3a2b20));
    c.disc(x, y, 6.0, rgb(0xe8dcc4));
    for k in 0..4 {
        let a = k as f64 * TAU / 4.0;
        c.set(
            cx + (a.sin() * 5.0).round() as i32,
            cy - (a.cos() * 5.0).round() as i32,
            rgb(0x3a2b20),
        );
    }
    // Ten past ten, as clocks in pictures always are.
    c.line((x, y), (x + 3.5, y - 2.0), rgb(0x2a2a2a));
    c.line((x, y), (x - 2.5, y - 1.5), rgb(0x2a2a2a));
    let tick = (phase(frame, 1) * 8.0).floor() / 8.0 * TAU;
    c.line(
        (x, y),
        (x + tick.sin() * 5.0, y - tick.cos() * 5.0),
        rgb(0xc0392b),
    );
    c.set(cx, cy, rgb(0x2a2a2a));
}

/// A framed poster: a little sunset over the sea.
fn poster(c: &mut Canvas, x: i32, y: i32) {
    c.rect(x, y, 16, 20, rgb(0x2a2a2a));
    c.gradient(
        x + 1,
        x + 15,
        y + 1,
        y + 13,
        &[rgb(0x2a1b4d), rgb(0xd1495b), rgb(0xf6a96c)],
    );
    c.disc(x as f64 + 8.0, y as f64 + 12.0, 3.5, rgb(0xffe39a));
    c.gradient(
        x + 1,
        x + 15,
        y + 13,
        y + 19,
        &[rgb(0x1d3b5a), rgb(0x0f2235)],
    );
    for k in 0..3 {
        c.rect(x + 5 + k, y + 14 + k * 2, 6 - k * 2, 1, rgb(0xf6c28b));
    }
}

/// A smaller frame: a pixel cat portrait, a nod to the one asleep on the desk.
fn poster_small(c: &mut Canvas, x: i32, y: i32) {
    c.rect(x, y, 11, 12, rgb(0x6b4632));
    c.rect(x + 1, y + 1, 9, 10, rgb(0xd8cfc0));
    let fur = rgb(0x3a3542);
    c.rect(x + 3, y + 5, 5, 4, fur);
    c.set(x + 3, y + 4, fur);
    c.set(x + 7, y + 4, fur);
    c.set(x + 4, y + 6, rgb(0xe9c46a));
    c.set(x + 6, y + 6, rgb(0xe9c46a));
}

/// A small frame holding a pressed leaf, so the gallery is not all cats.
fn leaf_print(c: &mut Canvas, x: i32, y: i32) {
    c.rect(x, y, 11, 12, rgb(0x2a2a2a));
    c.rect(x + 1, y + 1, 9, 10, rgb(0xe8dcc4));
    c.rect(x + 5, y + 3, 1, 7, rgb(0x3d6b3a));
    for (dx, dy) in [
        (-1, 4),
        (-2, 5),
        (1, 3),
        (2, 4),
        (-1, 6),
        (-2, 7),
        (1, 6),
        (2, 7),
        (0, 2),
    ] {
        c.set(x + 5 + dx, y + dy, if dy < 5 { LEAF_LIT } else { LEAF });
    }
}

/// Bulbs strung in shallow swags along absolute row `y`, twinkling.
fn fairy_lights(c: &mut Canvas, frame: u64, x0: i32, x1: i32, y: i32) {
    let colors = [0xffd27a, 0xff7eb6, 0x7ee0d6, 0xffa75e];
    let span = 28;
    for x in x0..x1 {
        let t = (x - x0).rem_euclid(span) as f64 / span as f64;
        let sag = (1.0 - (2.0 * t - 1.0).powi(2)) * 3.0;
        let wire_y = y + sag.round() as i32;
        c.set(x, wire_y, rgb(0x2a2440));
        if (x - x0).rem_euclid(5) == 2 {
            let seed = (x - x0) as u64;
            let color = rgb(colors[(seed / 5 % colors.len() as u64) as usize]);
            let twinkle = wave(frame, 1 + hash(seed) % 3, unit(seed + 50));
            let lit = mix(mix(color, rgb(0x000000), 0.5), color, twinkle);
            c.set(x, wire_y + 1, lit);
            c.glow(
                x as f64 + 0.5,
                wire_y as f64 + 1.5,
                3.0,
                color,
                0.18 * twinkle,
            );
        }
    }
}

/// A pendant lamp at core `x`, hanging from `ceiling` down to `bottom`, with a
/// warm halo and the gentlest sway.
fn pendant(c: &mut Canvas, frame: u64, x: i32, ceiling: i32, bottom: i32) {
    let sway = (phase(frame, 1) * TAU).sin() * 0.8;
    let bx = x as f64 + sway;
    c.line(
        (x as f64, ceiling as f64),
        (bx, (bottom - 4) as f64),
        rgb(0x0d0a1a),
    );
    let sx = bx.round() as i32;
    c.glow(bx + 0.5, bottom as f64 + 2.0, 14.0, WARM, 0.22);
    c.rect(sx - 2, bottom - 4, 5, 1, rgb(0x2b2b33));
    c.rect(sx - 3, bottom - 3, 7, 2, rgb(0x2f6b5e));
    c.rect(sx - 4, bottom - 1, 9, 1, rgb(0x3c8a79));
    c.rect(sx - 1, bottom, 3, 1, rgb(0xfff1c2));
}

/// A record player on the desk at absolute `x`, the record turning.
fn record_player(c: &mut Canvas, frame: u64, x: i32, desk: i32) {
    c.rect(x, desk - 4, 23, 4, rgb(0x6b4a3a));
    c.rect(x, desk - 4, 23, 1, rgb(0x87604c));
    c.rect(x + 1, desk - 5, 21, 1, rgb(0x2b2b33));
    // The platter, seen low: an ellipse of grooves around a red label whose
    // mark goes round, twice per loop.
    let (cx, cy) = (x as f64 + 9.5, (desk - 6) as f64);
    for dy in -2i32..=1 {
        for dx in -8i32..=8 {
            let e = (dx as f64 / 8.5).powi(2) + (dy as f64 / 2.5).powi(2);
            if e <= 1.0 {
                let ring = (e.sqrt() * 5.0) as i32;
                let groove = if ring % 2 == 0 { 0x262630 } else { 0x141418 };
                c.set(x + 9 + dx, desk - 6 + dy, rgb(groove));
            }
        }
    }
    c.rect(x + 8, desk - 7, 3, 2, rgb(0xd1495b));
    let a = phase(frame, 2) * TAU;
    c.set(
        (cx + a.cos() * 5.0).round() as i32,
        (cy + a.sin() * 1.2).round() as i32,
        rgb(0x6a6a7a),
    );
    // Tonearm.
    c.line(
        ((x + 20) as f64, (desk - 9) as f64),
        ((x + 14) as f64, (desk - 7) as f64),
        rgb(0xc8c8d0),
    );
    c.rect(x + 19, desk - 10, 3, 2, rgb(0x2b2b33));
    // Notes drifting up from it, now and then.
    for k in 0..2u64 {
        let rise = phase_at(frame, 1, k as f64 * 0.5);
        if rise < 0.6 {
            let nx = x + 6 + k as i32 * 7 + ((rise * TAU * 2.0).sin() * 1.5).round() as i32;
            let ny = desk - 11 - (rise * 14.0) as i32;
            let alpha = 1.0 - rise / 0.6;
            c.blend(nx, ny, rgb(0xe9c46a), alpha);
            c.blend(nx + 1, ny - 1, rgb(0xe9c46a), alpha);
            c.blend(nx + 1, ny - 2, rgb(0xe9c46a), alpha);
            c.blend(nx - 1, ny, rgb(0xe9c46a), alpha);
        }
    }
}

/// A laptop at absolute `x`, its screen scrolling through code.
fn laptop(c: &mut Canvas, frame: u64, x: i32, desk: i32) {
    let (sx, sy, sw, sh) = (x + 2, desk - 13, 17, 11);
    c.glow(
        (sx + sw / 2) as f64,
        (sy + sh / 2) as f64,
        14.0,
        rgb(0x6fa8ff),
        0.18,
    );
    c.rect(sx - 1, sy - 1, sw + 2, sh + 2, rgb(0x2b2b33));
    c.rect(sx, sy, sw, sh, rgb(0x0f1626));
    // Lines of "code": coloured runs, scrolling up a whole page per loop.
    let colors = [0x7ee0d6, 0xff7eb6, 0xe9c46a, 0x9aa8c7];
    let scroll = (phase(frame, 1) * 16.0) as i32;
    for row in 0..sh / 2 {
        let line = (row + scroll) as u64;
        let mut cx = sx + 1 + (hash(line % 16) % 3) as i32;
        let mut part = 0;
        while cx < sx + sw - 2 && part < 3 {
            let len = 2 + (hash(line % 16 * 10 + part) % 5) as i32;
            let len = len.min(sx + sw - 1 - cx);
            let color = rgb(colors[(hash(line % 16 + part * 3) % 4) as usize]);
            c.rect(cx, sy + 1 + row * 2, len, 1, color);
            cx += len + 1;
            part += 1;
        }
    }
    // Base and keyboard.
    c.rect(x, desk - 2, 21, 2, rgb(0x9aa0aa));
    c.rect(x + 3, desk - 2, 15, 1, rgb(0x6e7480));
}

/// A potted cactus at absolute `x`, its shape picked by `seed`.
fn cactus(c: &mut Canvas, frame: u64, x: i32, desk: i32, seed: u64) {
    let pot = [0xb5603c, 0xd8cfc0, 0x3b5a7a, 0x8a5a9e][(seed % 4) as usize];
    c.rect(x, desk - 4, 6, 4, rgb(pot));
    c.rect(x, desk - 4, 6, 1, mix(rgb(pot), rgb(0xffffff), 0.2));
    let height = 4 + (seed / 4 % 5) as i32;
    c.rect(x + 2, desk - 4 - height, 2, height, LEAF);
    c.rect(x + 2, desk - 4 - height, 1, height, LEAF_LIT);
    if seed.is_multiple_of(2) {
        c.rect(x, desk - 7, 2, 1, LEAF);
        c.rect(x, desk - 9, 1, 2, LEAF);
    }
    if seed.is_multiple_of(3) {
        c.rect(x + 4, desk - 8, 2, 1, LEAF);
        c.rect(x + 5, desk - 10, 1, 2, LEAF);
    }
    // Some of them flower, the flower pulsing gently.
    if seed.is_multiple_of(5) {
        let glow = 0.6 + 0.4 * wave(frame, 1, unit(seed));
        c.set(
            x + 2,
            desk - 5 - height,
            mix(rgb(0x6a2040), rgb(0xff7eb6), glow),
        );
    }
}

// --------------------------------------------------------------- the drive --

fn synthwave_drive(c: &mut Canvas, frame: u64) {
    let (ox, oy) = core_origin(c);
    let (w, h) = (c.width as i32, c.height as i32);
    let horizon = oy + 32;
    c.origin(0, 0);

    // Sky: the core's sunset band, with deeper night above it on a tall canvas.
    c.gradient(0, w, 0, oy.max(1), &[rgb(0x05010c), rgb(0x0e0220)]);
    c.gradient(
        0,
        w,
        oy.max(0),
        horizon,
        &[rgb(0x0e0220), rgb(0x2a0a52), rgb(0x7a1a7a), rgb(0xff3d7f)],
    );

    // Stars across all of it, each twinkling at its own whole rate. Density
    // follows the sky's area so a big terminal is not sparser.
    let sky_rows = (horizon - 14).max(1);
    let stars = (w * sky_rows / 55).max(20) as u64;
    for i in 0..stars {
        let x = (hash(i) % w as u64) as i32;
        let y = (hash(i + 50_000) % sky_rows as u64) as i32;
        let glow = wave(frame, 1 + hash(i + 90_000) % 3, unit(i + 130_000));
        c.blend(x, y, rgb(0xfff4ff), 0.2 + glow * 0.8);
    }

    // A shooting star, once a loop, across the upper sky.
    let streak = phase(frame, 1);
    if streak < 0.15 {
        let t = streak / 0.15;
        let (x0, y0) = (w as f64 * 0.15, (sky_rows as f64 * 0.15).max(2.0));
        let (x1, y1) = (w as f64 * 0.55, (sky_rows as f64 * 0.45).max(6.0));
        let (hx, hy) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
        for k in 0..10 {
            let back = k as f64 * 1.2;
            let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
            let (px, py) = (hx - (x1 - x0) / len * back, hy - (y1 - y0) / len * back);
            let fade = (1.0 - k as f64 / 10.0) * (1.0 - t * 0.6);
            c.blend(px.round() as i32, py.round() as i32, rgb(0xffffff), fade);
        }
    }

    // The sun: a gradient disc cut by bands that drift down and thicken.
    c.origin(ox, oy);
    let (cx, cy, r) = (48.0, 29.0, 17.0);
    let drift = phase(frame, 2) * 4.0;
    for y in (cy - r) as i32..32 {
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
    c.glow(cx, cy, r + 10.0, rgb(0xff4f9a), 0.18);

    // Mountains across the whole width: peaks placed by hashing their slot,
    // kept low in the middle so the sun shows between them.
    c.origin(0, 0);
    for (layer, slot, body, rim) in [
        (0u64, 17, rgb(0x3a1263), rgb(0xb04fd8)),
        (1u64, 13, rgb(0x1c0733), rgb(0xff4fd8)),
    ] {
        let mut peaks = Vec::new();
        let first = (-ox).div_euclid(slot) - 2;
        let last = (w - ox).div_euclid(slot) + 2;
        for k in first..=last {
            let seed = hash_at(k, 1000 + layer);
            let centre = (k * slot + (seed % slot as u64) as i32) as f64;
            let height = if layer == 0 { 6.0 } else { 4.0 } + (seed / 17 % 6) as f64;
            let half = height * (1.2 + (seed / 101 % 5) as f64 * 0.15);
            peaks.push((centre, height, half));
        }
        for x in 0..w {
            let core_x = (x - ox) as f64;
            let open = ((core_x - 48.0).abs() / 26.0).clamp(0.25, 1.0);
            let height = peaks
                .iter()
                .map(|&(cx, hgt, half)| hgt * (1.0 - (core_x - cx).abs() / half))
                .fold(0.0_f64, f64::max)
                * open;
            if height < 0.5 {
                continue;
            }
            let jag = (hash_at(x - ox, 7 + layer) % 3) as f64 * 0.5;
            let top = horizon - (height + jag).round() as i32;
            for y in top..horizon {
                c.set(x, y, body);
            }
            c.set(x, top, rim);
        }
    }

    // Ground, and the grid rolling towards the viewer.
    c.gradient(0, w, horizon, h, &[rgb(0x1a0438), rgb(0x0b0220)]);
    let depth = (h - horizon) as f64;
    let roll = phase(frame, 4);
    for k in 0..7 {
        let z = k as f64 + 1.0 - roll;
        if z <= 0.0 {
            continue;
        }
        let y = horizon as f64 + depth / z;
        // Lines this far out would only pile into a band at the horizon.
        if y < horizon as f64 + 2.5 {
            continue;
        }
        let fade = (1.0 / z).sqrt();
        for x in 0..w {
            c.blend(x, y.round() as i32, rgb(0xff2fd1), 0.2 + 0.8 * fade);
        }
    }
    // Lines out of the vanishing point, stepped along their longer axis so a
    // shallow one is a continuous stroke rather than a row of dots. Enough of
    // them to reach both edges however wide the canvas is.
    let vx = (ox + 48) as f64;
    let reach = ((w as f64).max(1.0) / 13.0).ceil() as i32 + 2;
    for i in -reach..=reach {
        let (x0, y0) = (vx, horizon as f64);
        let (x1, y1) = (vx + i as f64 * 13.0 * depth / 22.0, h as f64);
        let steps = (x1 - x0).abs().max(y1 - y0).ceil().max(1.0) as i32;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
            if y < horizon as f64 + 3.0 {
                continue;
            }
            c.blend(
                x.round() as i32,
                y.round() as i32,
                rgb(0xff2fd1),
                0.05 + 0.95 * t.powf(1.6),
            );
        }
    }
    c.rect(0, horizon, w, 1, rgb(0xff6ad5));

    // Palms: the core's pair, and more down each side, alternating lean.
    let mut roots = vec![(ox + 9, 1.0), (ox + 86, -1.0)];
    let mut k = 1;
    loop {
        let (l, r) = (ox + 9 - k * 30, ox + 86 + k * 30);
        if l < -8 && r > w + 8 {
            break;
        }
        let lean = if k % 2 == 0 { 1.0 } else { -1.0 };
        roots.push((l, lean));
        roots.push((r, -lean));
        k += 1;
    }
    for (root, lean) in roots {
        palm(c, frame, root as f64, lean, h, oy + 22);
    }

    // The car, seen from behind, bobbing on the road, tail lights pulsing.
    c.origin(ox, h - 54);
    car(c, frame);
}

fn palm(c: &mut Canvas, frame: u64, root: f64, lean: f64, bottom: i32, crown: i32) {
    let silhouette = rgb(0x0c0016);
    // The trunk bows out and back, as palms do.
    let top = (root + lean * 6.0, crown as f64);
    let base = (bottom - 1) as f64;
    let steps = 40;
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let x = root + (top.0 - root) * t * t;
        let y = base + (top.1 - base) * t;
        let width = if t < 0.6 { 2 } else { 1 };
        for w in 0..width {
            c.set(x.round() as i32 + w, y.round() as i32, silhouette);
        }
    }
    // Fronds arc out and droop, thick at the crown and thin at the tips;
    // they stir a little, once per loop.
    let stir = (phase_at(frame, 1, root.rem_euclid(7.0) / 7.0) * TAU).sin() * 0.06;
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

/// A sports car from behind, at the bottom of the core, rims catching neon.
fn car(c: &mut Canvas, frame: u64) {
    let bob = if phase(frame, 8) < 0.5 { 0 } else { 1 };
    let (x, y) = (35, 41 + bob);
    let body = rgb(0x0a0012);
    let rim = rgb(0xff4fd8);
    // Roof and rear window.
    c.rect(x + 7, y, 12, 1, rim);
    c.rect(x + 6, y + 1, 14, 3, body);
    c.rect(x + 8, y + 1, 10, 2, rgb(0x2a0a52));
    // Body, wider and low.
    c.rect(x + 1, y + 4, 24, 5, body);
    c.rect(x, y + 5, 26, 3, body);
    c.rect(x + 1, y + 4, 24, 1, rim);
    // Tail lights, a band across, pulsing.
    let glow = 0.55 + 0.45 * wave(frame, 2, 0.0);
    let red = mix(rgb(0x5a0010), rgb(0xff2a4a), glow);
    c.rect(x + 2, y + 6, 7, 1, red);
    c.rect(x + 17, y + 6, 7, 1, red);
    c.rect(x + 9, y + 6, 8, 1, mix(red, body, 0.5));
    c.glow(
        x as f64 + 5.5,
        y as f64 + 6.5,
        6.0,
        rgb(0xff2a4a),
        0.3 * glow,
    );
    c.glow(
        x as f64 + 20.5,
        y as f64 + 6.5,
        6.0,
        rgb(0xff2a4a),
        0.3 * glow,
    );
    // Plate and wheels.
    c.rect(x + 11, y + 7, 4, 1, rgb(0xe8e8f0));
    c.rect(x + 2, y + 9, 4, 2, body);
    c.rect(x + 20, y + 9, 4, 2, body);
}

// ------------------------------------------------------------------ widget --

/// Draws a scene filling `area`: composed for the area's shape at the pixel
/// size [`fit`] picks, centred in the few cells that division leaves over.
pub struct SceneWidget {
    scene: Scene,
    frame: u64,
}

impl SceneWidget {
    pub fn new(scene: Scene, frame: u64) -> Self {
        Self { scene, frame }
    }
}

impl Widget for SceneWidget {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let (scale, cw, ch) = fit(area.width, area.height);
        let canvas = self.scene.render(self.frame, cw, ch);

        let (draw_w, draw_h) = (cw * scale, ch * scale);
        let rows = draw_h.div_ceil(2);
        let x0 = area.x as usize + (area.width as usize - draw_w) / 2;
        let y0 = area.y as usize + (area.height as usize - rows) / 2;

        let sample = |px: usize, py: usize| -> Color {
            if py >= draw_h {
                return Color::Reset;
            }
            let [r, g, b] = canvas.get(px / scale, py / scale);
            Color::Rgb(r, g, b)
        };
        for row in 0..rows {
            for col in 0..draw_w {
                let Some(cell) = buf.cell_mut(((x0 + col) as u16, (y0 + row) as u16)) else {
                    continue;
                };
                cell.set_char('▀');
                cell.set_style(
                    Style::new()
                        .fg(sample(col, row * 2))
                        .bg(sample(col, row * 2 + 1)),
                );
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use ratatui::buffer::Buffer;

    /// Canvas shapes worth checking: the bare core, a typical full-screen
    /// terminal at 1×, a very wide one, a tall one, and one smaller than the
    /// core.
    const SIZES: [(usize, usize); 5] = [(96, 54), (180, 88), (300, 70), (110, 120), (60, 30)];

    fn changed(a: &Canvas, b: &Canvas) -> usize {
        (0..a.height())
            .flat_map(|y| (0..a.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| a.get(x, y) != b.get(x, y))
            .count()
    }

    /// The seam: every scene, at every size and any frame, is the same picture
    /// one loop on.
    #[test]
    fn a_scene_loops_exactly() {
        for scene in Scene::ALL {
            for (w, h) in SIZES {
                for frame in [0, 1, 37, 80, 159] {
                    assert!(
                        scene.render(frame, w, h) == scene.render(frame + FRAMES, w, h),
                        "{scene:?} at {w}x{h}, frame {frame}, differs one loop later"
                    );
                }
            }
        }
    }

    /// …and it does move, all through the loop.
    ///
    /// Sampled at frames that divide the loop unevenly: at exactly a half or
    /// a quarter, parts running 2 or 4 cycles per loop are back where they
    /// started by design, and the picture can look nearly still.
    #[test]
    fn a_scene_is_animated() {
        for scene in Scene::ALL {
            let a = scene.render(0, WIDTH, HEIGHT);
            for later in [13, 57, 101] {
                let moved = changed(&a, &scene.render(later, WIDTH, HEIGHT));
                assert!(
                    moved > 40,
                    "{scene:?} barely moved by frame {later}: {moved}"
                );
            }
        }
    }

    /// Consecutive frames differ only a little: motion, not flashing.
    ///
    /// Checked from the core's size up. On a canvas smaller than the core the
    /// rolling grid is most of the picture, and its ordinary motion alone
    /// crosses this line — the seam measured no worse there than any other
    /// grid step.
    #[test]
    fn consecutive_frames_change_gently() {
        for scene in Scene::ALL {
            for (w, h) in SIZES
                .into_iter()
                .filter(|&(w, h)| w >= WIDTH && h >= HEIGHT)
            {
                for frame in [0, 50, FRAMES - 1] {
                    let a = scene.render(frame, w, h);
                    let moved = changed(&a, &scene.render(frame + 1, w, h));
                    assert!(
                        moved < w * h / 4,
                        "{scene:?} at {w}x{h}, frame {frame}: {moved} pixels changed"
                    );
                }
            }
        }
    }

    /// Growing the canvas adds room around the composition; it does not move
    /// or rescale it. The desk's window frame is a fixed pixel of the core.
    #[test]
    fn the_core_keeps_its_place_at_any_size() {
        for (w, h) in SIZES
            .into_iter()
            .filter(|&(w, h)| w >= WIDTH && h >= HEIGHT)
        {
            let canvas = Scene::RainyDesk.render(0, w, h);
            // The frame's left side runs from the core's row 3 to 33; row 5
            // is on the canvas even at the lowest anchor.
            let (ox, oy) = desk_origin(&canvas);
            assert_eq!(
                canvas.get((ox + 10) as usize, (oy + 5) as usize),
                rgb(0x4a3328),
                "window frame moved at {w}x{h}"
            );
        }
    }

    /// The extra room is furnished, not left as blank wall: both sides of the
    /// core carry plenty of clearly bright pixels — books, pots, frames,
    /// screens; neon and palms on the drive. Wall shades never get this bright;
    /// with the side furniture taken out, the desk measured 4–16 such pixels a
    /// side (the odd fairy light), against hundreds furnished.
    #[test]
    fn extra_room_is_furnished() {
        for scene in Scene::ALL {
            for (w, h) in [(180usize, 88usize), (300, 70)] {
                let canvas = scene.render(0, w, h);
                let ox = (w - WIDTH) / 2;
                let bright = |xs: std::ops::Range<usize>| {
                    xs.flat_map(|x| (0..h).map(move |y| (x, y)))
                        .filter(|&(x, y)| canvas.get(x, y).iter().max().is_some_and(|&c| c >= 0x90))
                        .count()
                };
                let (left, right) = (bright(0..ox), bright(ox + WIDTH..w));
                assert!(
                    left >= 100 && right >= 100,
                    "{scene:?} at {w}x{h}: {left} bright pixels left of the core, {right} right"
                );
            }
        }
    }

    /// The cat sits at least a third of the way up, at every size — found in
    /// the picture itself, as the lowest row of its fur under the window.
    #[test]
    fn the_cat_sits_at_least_a_third_up() {
        let fur = rgb(0x3a3542);
        for (w, h) in SIZES
            .into_iter()
            .chain([(200, 160), (96, 100)])
            .filter(|&(w, h)| w >= WIDTH && h >= HEIGHT)
        {
            for frame in [0, 40] {
                let canvas = Scene::RainyDesk.render(frame, w, h);
                let ox = (w - WIDTH) / 2;
                let lowest = (0..h)
                    .rev()
                    .find(|&y| (ox + 17..ox + 44).any(|x| canvas.get(x, y) == fur))
                    .unwrap_or_else(|| panic!("no cat at {w}x{h}"));
                let below = h - 1 - lowest;
                assert!(
                    below * 3 >= h,
                    "{w}x{h}: only {below} rows under the cat, need {}",
                    h.div_ceil(3)
                );
            }
        }
    }

    /// Under the desk is a room, not more desk: a floor along the bottom with
    /// the rug on it.
    #[test]
    fn there_is_a_room_under_the_desk() {
        for (w, h) in [(180usize, 88usize), (110, 120), (300, 70)] {
            let canvas = Scene::RainyDesk.render(0, w, h);
            let bottom: Vec<_> = (0..w).map(|x| canvas.get(x, h - 2)).collect();
            assert!(
                bottom.contains(&rgb(0x7a2e43)) || bottom.contains(&rgb(0xc46a80)),
                "{w}x{h}: no rug along the bottom"
            );
            assert!(
                !bottom.iter().all(|&p| p == rgb(0x4a2f22)),
                "{w}x{h}: the bottom is still desk"
            );
        }
    }

    /// The wall reaches the top on a tall canvas: no row there is left black.
    #[test]
    fn the_wall_runs_to_the_ceiling() {
        let canvas = Scene::RainyDesk.render(0, 110, 120);
        for y in 0..60 {
            assert_ne!(canvas.get(55, y), [0, 0, 0], "row {y} is unpainted");
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

    #[test]
    fn the_pixel_size_is_the_largest_that_fits_the_core() {
        // A typical full-screen terminal: under 2× the core, so 1×, and the
        // whole area is canvas.
        assert_eq!(fit(180, 44), (1, 180, 88));
        // Exactly twice the core.
        assert_eq!(fit(192, 54), (2, 96, 54));
        // Room for 3× across but only 2× down: 2×, canvas wider than the core.
        assert_eq!(fit(300, 60), (2, 150, 60));
        // Smaller than the core: still 1×, a smaller canvas.
        assert_eq!(fit(40, 10), (1, 40, 20));
    }

    fn draw(width: u16, height: u16, scene: Scene) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        SceneWidget::new(scene, 0).render(area, &mut buf);
        buf
    }

    /// Regression: the scene was a small island, drawn at the largest whole
    /// scale of a fixed canvas. Every cell of the area is now picture.
    #[test]
    fn the_scene_fills_the_whole_area() {
        for (w, h) in [(180u16, 44u16), (150, 60), (300, 70), (97, 28)] {
            let buf = draw(w, h, Scene::RainyDesk);
            let (scale, cw, ch) = fit(w, h);
            // Whatever division leaves over is under one pixel each way.
            let (blank_w, blank_h) = (w as usize - cw * scale, h as usize * 2 - ch * scale);
            assert!(blank_w < scale && blank_h < scale.max(2), "{w}x{h}");
            for (x, y) in [
                (blank_w as u16 / 2, 0),
                (w - 1 - (blank_w as u16).div_ceil(2), h - 1),
            ] {
                assert_eq!(buf[(x, y)].symbol(), "▀", "{w}x{h}: cell {x},{y} blank");
            }
        }
    }

    /// At a whole scale above one, a pixel is a crisp block: both halves of a
    /// cell are the same source pixel, and its neighbour across is too.
    #[test]
    fn pixels_scale_up_as_crisp_blocks() {
        let buf = draw(192, 54, Scene::RainyDesk);
        let canvas = Scene::RainyDesk.render(0, 96, 54);
        for (col, row) in [(0u16, 0u16), (37, 10), (191, 53)] {
            let [r, g, b] = canvas.get(col as usize / 2, row as usize);
            let cell = &buf[(col, row)];
            assert_eq!(cell.fg, Color::Rgb(r, g, b), "at {col},{row}");
            assert_eq!(cell.bg, Color::Rgb(r, g, b), "at {col},{row}");
        }
    }

    #[test]
    fn tiny_and_empty_areas_do_not_panic() {
        for scene in Scene::ALL {
            for (w, h) in [(0, 0), (1, 1), (10, 3), (3, 40), (500, 2), (2, 300)] {
                draw(w, h, scene);
            }
        }
    }

    /// Writes frames of each scene at a few terminal shapes as PNGs, enlarged:
    /// `NAVIDROME_RS_SCENE_DIR=/tmp/x cargo test preview_scenes -- --ignored`
    #[test]
    #[ignore = "writes image files for looking at"]
    fn preview_scenes() {
        let dir = std::env::var("NAVIDROME_RS_SCENE_DIR").expect("set NAVIDROME_RS_SCENE_DIR");
        for scene in Scene::ALL {
            for (w, h) in [(96, 54), (180, 88), (300, 70), (110, 120)] {
                for frame in [0, FRAMES / 3] {
                    let canvas = scene.render(frame, w, h);
                    let zoom = (1200 / w).max(1);
                    let image =
                        image::RgbImage::from_fn((w * zoom) as u32, (h * zoom) as u32, |x, y| {
                            image::Rgb(canvas.get(x as usize / zoom, y as usize / zoom))
                        });
                    image
                        .save(format!("{dir}/{scene:?}-{w}x{h}-{frame:03}.png"))
                        .unwrap();
                }
            }
        }
    }
}
