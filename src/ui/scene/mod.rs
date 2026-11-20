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

mod cafe;
mod cafe_street;
mod desk;
mod drive;
mod flat;
mod parts;
mod study;
mod train;

use cafe::coffee_bar;
use cafe_street::cafe_street;
use desk::rainy_desk;
use drive::synthwave_drive;
use flat::sunset_flat;
use study::night_study;
use train::night_train;

/// The core composition, in pixels: 16:9, small enough to read as pixel art.
pub const WIDTH: usize = 96;
pub const HEIGHT: usize = 54;

/// Frames per second the scene is designed for. The app redraws on its 50ms
/// tick, which is exactly this.
pub const FPS: u64 = 20;

/// Frames in one loop: eight seconds.
pub const FRAMES: u64 = 8 * FPS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scene {
    /// Night, rain on the window, a lamp, a mug, a sleeping cat.
    RainyDesk,
    /// A counter by a café window: a latte, and a rainy street outside with
    /// people under umbrellas and a tram going by.
    CafeWindow,
    /// Behind the bar of a coffee shop: a barista at the espresso machine,
    /// a chalkboard menu, customers at tables along the room.
    CoffeeBar,
    /// A carriage at night: passengers on the bench, and the country going
    /// by outside in layers, each at its own speed.
    NightTrain,
    /// A flat at sunset: a dog asleep on the rug, a sofa, a beam of low sun
    /// across the room with dust turning in it.
    SunsetFlat,
    /// Somebody at their desk from behind, writing and nodding along, snow
    /// falling past a big window, a goldfish going round its bowl.
    NightStudy,
    /// Neon sun, mountains, a grid rolling towards you, palms, a car.
    SynthwaveDrive,
}

/// Words in the music that call for a scene, checked in order: the first
/// scene with one of its words in the text wins. Anything else — lofi
/// included — gets the rainy desk.
const KEYWORDS: &[(Scene, &[&str])] = &[
    (
        Scene::SynthwaveDrive,
        &[
            "synth",
            "retro",
            "outrun",
            "vapor",
            "nightwave",
            "80s",
            "neon",
            "cyber",
        ],
    ),
    (
        Scene::NightStudy,
        &["study", "focus", "piano", "snow", "winter"],
    ),
    (
        Scene::NightTrain,
        &["train", "travel", "journey", "ambient"],
    ),
    (
        Scene::SunsetFlat,
        &["sunset", "sunday", "home", "cozy", "soul"],
    ),
    (
        Scene::CoffeeBar,
        &["coffee", "cafe", "café", "brunch", "acoustic", "morning"],
    ),
    (Scene::CafeWindow, &["jazz", "rain", "bossa", "city"]),
];

impl Scene {
    pub const ALL: [Scene; 7] = [
        Scene::RainyDesk,
        Scene::CafeWindow,
        Scene::CoffeeBar,
        Scene::NightTrain,
        Scene::SunsetFlat,
        Scene::NightStudy,
        Scene::SynthwaveDrive,
    ];

    /// The scene for what is playing, from the words in its title, artist
    /// and album — a station is only a name and a URL on the server, so its
    /// directory tags are not there to read. The first scene in [`KEYWORDS`]
    /// with a word in the text wins; anything else gets the desk.
    pub fn for_words(text: &str) -> Self {
        let text = text.to_lowercase();
        KEYWORDS
            .iter()
            .find(|(_, words)| words.iter().any(|word| text.contains(word)))
            .map_or(Scene::RainyDesk, |&(scene, _)| scene)
    }

    /// What the viewer calls it.
    pub fn name(self) -> &'static str {
        match self {
            Scene::RainyDesk => "Rainy desk",
            Scene::CafeWindow => "Cafe window",
            Scene::CoffeeBar => "Coffee bar",
            Scene::NightTrain => "Night train",
            Scene::SunsetFlat => "Sunset flat",
            Scene::NightStudy => "Night study",
            Scene::SynthwaveDrive => "Synthwave drive",
        }
    }

    /// The scene `by` steps along from this one, wrapping either way.
    pub fn turned(self, by: i32) -> Self {
        let here = Self::ALL.iter().position(|&s| s == self).unwrap_or(0) as i32;
        Self::ALL[(here + by).rem_euclid(Self::ALL.len() as i32) as usize]
    }

    /// One frame on a `width`×`height` canvas. The core composition keeps its
    /// size and sits bottom-centre; the rest is furnished to fit.
    pub fn render(self, frame: u64, width: usize, height: usize) -> Canvas {
        let mut canvas = Canvas::new(width.max(1), height.max(1));
        match self {
            Scene::RainyDesk => rainy_desk(&mut canvas, frame),
            Scene::CafeWindow => cafe_street(&mut canvas, frame),
            Scene::CoffeeBar => coffee_bar(&mut canvas, frame),
            Scene::NightTrain => night_train(&mut canvas, frame),
            Scene::SunsetFlat => sunset_flat(&mut canvas, frame),
            Scene::NightStudy => night_study(&mut canvas, frame),
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

    /// A filled ellipse.
    fn ellipse(&mut self, cx: f64, cy: f64, rx: f64, ry: f64, color: Rgb) {
        for y in (cy - ry).floor() as i32..=(cy + ry).ceil() as i32 {
            for x in (cx - rx).floor() as i32..=(cx + rx).ceil() as i32 {
                let (dx, dy) = ((x as f64 + 0.5 - cx) / rx, (y as f64 + 0.5 - cy) / ry);
                if dx * dx + dy * dy <= 1.0 {
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

/// Like [`core_origin`], but raised until the core's row `lowest` — the
/// bottom of its subject, the cat or the car — sits at least a third of the
/// height up from the bottom. What lies below the subject simply grows.
///
/// Never more than `crop` rows above the canvas top: on a canvas too short
/// to honour the third, the subject gets as high as that allows.
fn raised_origin(c: &Canvas, lowest: i32, crop: i32) -> (i32, i32) {
    let (ox, bottom_anchored) = core_origin(c);
    let h = c.height as i32;
    // Below row `oy + lowest` there must be at least `ceil(h / 3)` rows.
    let third_up = h - (h + 2) / 3 - (lowest + 1);
    (ox, bottom_anchored.min(third_up).max(-crop))
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
    use super::desk::desk_origin;
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

    /// Sharp changes between neighbouring pixels, per pixel, over columns
    /// `xs`: furniture and objects make them, plain walls and gradients do not.
    fn edge_density(c: &Canvas, xs: std::ops::Range<usize>) -> f64 {
        let (mut sharp, mut total) = (0usize, 0usize);
        for y in 0..c.height() {
            for x in xs.clone().filter(|&x| x + 1 < c.width()) {
                let (a, b) = (c.get(x, y), c.get(x + 1, y));
                let d: i32 = (0..3).map(|i| (a[i] as i32 - b[i] as i32).abs()).sum();
                total += 1;
                sharp += (d > 90) as usize;
            }
        }
        sharp as f64 / total.max(1) as f64
    }

    /// The extra room beside the core is furnished, not left as bare wall,
    /// in every scene. Measured on the sparsest side there is — the study's
    /// right, with a clock and a poster — 0.019; the same side with those
    /// taken out, 0.006. Most sides score 0.05 to 0.17.
    #[test]
    fn extra_room_is_furnished() {
        for scene in Scene::ALL {
            for (w, h) in [(180usize, 88usize), (300, 70)] {
                let canvas = scene.render(0, w, h);
                let ox = (w - WIDTH) / 2;
                let (left, right) = (
                    edge_density(&canvas, 0..ox),
                    edge_density(&canvas, ox + WIDTH..w),
                );
                assert!(
                    left >= 0.012 && right >= 0.012,
                    "{} at {w}x{h}: edge density {left:.3} left of the core, {right:.3} right",
                    scene.name()
                );
            }
        }
    }

    /// Every scene's subject — the cat, the cup, the barista, the passengers,
    /// the dog, the writer's desk, the car — sits at least a third of the way
    /// up a tall screen, like the cat and the car before them.
    #[test]
    fn every_subject_sits_at_least_a_third_up() {
        type Origin = fn(&Canvas) -> (i32, i32);
        let anchors: [(Scene, Origin, i32); 7] = [
            (Scene::RainyDesk, super::desk::desk_origin, 39),
            (Scene::CafeWindow, super::cafe_street::cafe_origin, 40),
            (Scene::CoffeeBar, super::cafe::bar_origin, 44),
            (Scene::NightTrain, super::train::train_origin, 44),
            (Scene::SunsetFlat, super::flat::flat_origin, 47),
            (Scene::NightStudy, super::study::study_origin, 38),
            (Scene::SynthwaveDrive, super::drive::drive_origin, 52),
        ];
        assert_eq!(anchors.len(), Scene::ALL.len(), "every scene listed");
        for (scene, origin, lowest) in anchors {
            for (w, h) in [(180usize, 88usize), (110, 120), (200, 160)] {
                let canvas = Canvas::new(w, h);
                let (_, oy) = origin(&canvas);
                let below = h as i32 - 1 - (oy + lowest);
                assert!(
                    below * 3 >= h as i32,
                    "{} at {w}x{h}: {below} rows under the subject",
                    scene.name()
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

    /// The car, like the cat, sits at least a third of the way up wherever the
    /// canvas is tall enough to allow it without losing the sun — found in
    /// the picture as the lowest row of its body.
    #[test]
    fn the_car_sits_at_least_a_third_up() {
        let body = rgb(0x0a0012);
        for (w, h) in [(180usize, 88usize), (110, 120), (200, 160), (96, 100)] {
            for frame in [0, 10] {
                let canvas = Scene::SynthwaveDrive.render(frame, w, h);
                let ox = (w - WIDTH) / 2;
                let lowest = (0..h)
                    .rev()
                    .find(|&y| (ox + 35..ox + 61).any(|x| canvas.get(x, y) == body))
                    .unwrap_or_else(|| panic!("no car at {w}x{h}"));
                let below = h - 1 - lowest;
                assert!(
                    below * 3 >= h,
                    "{w}x{h} frame {frame}: only {below} rows under the car, need {}",
                    h.div_ceil(3)
                );
            }
        }
    }

    /// The pair at the café counter take turns: through a loop the bubble
    /// shows over each of them for a good while, never both at once, with
    /// pauses between.
    #[test]
    fn the_cafe_pair_take_turns_talking() {
        let (mut left, mut right, mut quiet) = (0, 0, 0);
        for frame in 0..FRAMES {
            let canvas = Scene::CafeWindow.render(frame, WIDTH, HEIGHT);
            let xs: Vec<usize> = (0..HEIGHT)
                .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
                .filter(|&(x, y)| canvas.get(x, y) == super::cafe_street::BUBBLE)
                .map(|(x, _)| x)
                .collect();
            match (xs.iter().min(), xs.iter().max()) {
                (None, _) | (_, None) => quiet += 1,
                // The pair sit at 20 and 40: a bubble is over one of them.
                (Some(&lo), Some(&hi)) => {
                    assert!(hi - lo < 12, "frame {frame}: two bubbles at once");
                    if hi < 33 {
                        left += 1;
                    } else if lo > 30 {
                        right += 1;
                    }
                }
            }
        }
        assert!(
            left >= 30 && right >= 30,
            "turns: {left} left, {right} right"
        );
        assert!(quiet >= 10, "no pauses: {quiet} quiet frames");
    }

    /// On a tall screen the coffee bar's floor fills with people, rows of
    /// them down from the counter to the front, not one row and a gap. In the
    /// band between, edge density measured 0.056 to 0.070 with the rows and
    /// 0.000 without.
    #[test]
    fn a_tall_coffee_bar_is_full_of_people() {
        for (w, h) in [(110usize, 120usize), (200, 160), (300, 100)] {
            let canvas = Scene::CoffeeBar.render(0, w, h);
            let (_, oy) = super::cafe::bar_origin(&canvas);
            let floor = (oy + 44) as usize;
            let (mut sharp, mut total) = (0usize, 0usize);
            for y in floor + 10..h - 14 {
                for x in 0..w - 1 {
                    let (a, b) = (canvas.get(x, y), canvas.get(x + 1, y));
                    let d: i32 = (0..3).map(|i| (a[i] as i32 - b[i] as i32).abs()).sum();
                    total += 1;
                    sharp += (d > 90) as usize;
                }
            }
            let density = sharp as f64 / total.max(1) as f64;
            assert!(
                density >= 0.02,
                "{w}x{h}: the middle floor is empty ({density:.3})"
            );
        }
    }

    /// The café window's bar stays bar-height however tall the screen: its
    /// front, from the ledge to the floor tiles, is never more than `BAR`.
    /// It once grew to a third of the screen and read as a wall.
    #[test]
    fn the_cafe_bar_stays_bar_height() {
        let tile_dark = rgb(0x2a2420);
        for (w, h) in [(180usize, 88usize), (110, 120), (200, 160)] {
            let canvas = Scene::CafeWindow.render(0, w, h);
            let (ox, oy) = super::cafe_street::cafe_origin(&canvas);
            let ledge = (oy + 40) as usize;
            // Down a column clear of stools and people, to the first tile.
            let x = (ox + 60) as usize;
            let tiles = (ledge..h)
                .find(|&y| canvas.get(x, y) == tile_dark || canvas.get(x, y) == rgb(0x3a322a));
            let tiles = tiles.unwrap_or_else(|| panic!("{w}x{h}: no floor below the bar"));
            assert!(
                tiles - ledge <= super::cafe_street::BAR as usize + 1,
                "{w}x{h}: the bar front is {} rows",
                tiles - ledge
            );
        }
    }

    /// Below the café window's bar, the floor fills with customers rather
    /// than standing empty. Edge density there measured 0.038 to 0.056 with
    /// the tables and 0.000 without.
    #[test]
    fn the_cafe_floor_has_customers() {
        for (w, h) in [(180usize, 88usize), (110, 120), (200, 160)] {
            let canvas = Scene::CafeWindow.render(0, w, h);
            let (_, oy) = super::cafe_street::cafe_origin(&canvas);
            let floor = (oy + 40 + super::cafe_street::BAR) as usize;
            let (mut sharp, mut total) = (0usize, 0usize);
            for y in floor + 2..h - 1 {
                for x in 0..w - 1 {
                    let (a, b) = (canvas.get(x, y), canvas.get(x + 1, y));
                    let d: i32 = (0..3).map(|i| (a[i] as i32 - b[i] as i32).abs()).sum();
                    total += 1;
                    sharp += (d > 90) as usize;
                }
            }
            let density = sharp as f64 / total.max(1) as f64;
            assert!(
                density >= 0.015,
                "{w}x{h}: the café floor is empty ({density:.3})"
            );
        }
    }

    /// The train's floor, without the riders, the conductor and our side,
    /// still has poles, lamp light and the things at people's feet: it measured
    /// 0.075 to 0.084. With them, 0.139 to 0.209.
    const AISLE_MIN: f64 = 0.11;

    /// The conductor works down the carriage: across the loop they get from
    /// one side to the other, and they stop along the way.
    #[test]
    fn the_conductor_walks_and_stops() {
        let (w, h) = (180usize, 88usize);
        let at = |frame| {
            let canvas = Scene::NightTrain.render(frame, w, h);
            let xs: Vec<usize> = (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .filter(|&(x, y)| canvas.get(x, y) == super::train::CAP_BAND)
                .map(|(x, _)| x)
                .collect();
            xs.first().copied()
        };
        let seen: Vec<usize> = (0..FRAMES).filter_map(at).collect();
        let (lo, hi) = (*seen.iter().min().unwrap(), *seen.iter().max().unwrap());
        assert!(
            hi - lo > w / 2,
            "the conductor stayed between {lo} and {hi}"
        );
        let still = (0..FRAMES - 1)
            .filter(|&f| at(f).is_some() && at(f) == at(f + 1))
            .count();
        assert!(still >= 20, "never stopped: {still} still frames");
    }

    /// Under the far bench the train's floor fills: people standing in the
    /// aisle, the conductor, our side's silhouettes along the bottom; see
    /// `AISLE_MIN` for how bare and busy measured.
    #[test]
    fn the_train_aisle_is_not_empty() {
        for (w, h) in [(180usize, 88usize), (110, 120), (200, 160)] {
            let canvas = Scene::NightTrain.render(0, w, h);
            let (_, oy) = super::train::train_origin(&canvas);
            let floor = (oy + 46) as usize;
            let (mut sharp, mut total) = (0usize, 0usize);
            for y in floor + 6..h - 1 {
                for x in 0..w - 1 {
                    let (a, b) = (canvas.get(x, y), canvas.get(x + 1, y));
                    let d: i32 = (0..3).map(|i| (a[i] as i32 - b[i] as i32).abs()).sum();
                    total += 1;
                    sharp += (d > 90) as usize;
                }
            }
            let density = sharp as f64 / total.max(1) as f64;
            assert!(
                density >= AISLE_MIN,
                "{w}x{h}: the aisle is empty ({density:.3})"
            );
            let shadows = (0..w)
                .filter(|&x| canvas.get(x, h - 1) == super::train::SHADOW)
                .count();
            assert!(shadows > w / 4, "{w}x{h}: nobody on our side ({shadows})");
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
    fn turning_wraps_both_ways_through_every_scene() {
        let start = Scene::RainyDesk;
        let n = Scene::ALL.len() as i32;
        assert_eq!(start.turned(0), start);
        assert_eq!(start.turned(1), Scene::ALL[1]);
        assert_eq!(start.turned(n), start, "a full turn comes back");
        assert_eq!(start.turned(-1), start.turned(n - 1), "backwards wraps");
        let seen: std::collections::HashSet<_> = (0..n).map(|k| start.turned(k)).collect();
        assert_eq!(seen.len(), Scene::ALL.len(), "every scene is reachable");
    }

    #[test]
    fn the_music_picks_the_scene() {
        for (text, scene) in [
            ("Nightwave Plaza", Scene::SynthwaveDrive),
            ("Esoterica — Synthwave", Scene::SynthwaveDrive),
            ("Late Night Jazz Lounge", Scene::CafeWindow),
            ("Morning Coffee Acoustic", Scene::CoffeeBar),
            ("Ambient Journey", Scene::NightTrain),
            ("Sunday Soul", Scene::SunsetFlat),
            ("Piano for Study", Scene::NightStudy),
            ("Lofi 24/7", Scene::RainyDesk),
            ("", Scene::RainyDesk),
        ] {
            assert_eq!(Scene::for_words(text), scene, "{text:?}");
        }
        // Every scene can be reached by some word, and the default by none.
        for scene in Scene::ALL {
            let reachable = scene == Scene::RainyDesk
                || KEYWORDS
                    .iter()
                    .any(|&(s, words)| s == scene && !words.is_empty());
            assert!(reachable, "{} has no words", scene.name());
        }
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
