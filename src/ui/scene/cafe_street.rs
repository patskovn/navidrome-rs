//! The café window: a counter by the glass, a latte steaming on it, and a
//! rainy street outside — shop fronts, people under umbrellas, a tram going
//! by — built out to fill the screen.

use std::f64::consts::TAU;

use super::parts::{Look, look, rain, sprite, street, walker};
use super::*;

/// The scene's anchor. Its subject is the cup on the counter, whose base is
/// the core's row 40; the counter front grows beneath it.
pub(super) fn cafe_origin(c: &Canvas) -> (i32, i32) {
    raised_origin(c, 40, 4)
}

/// Core rows: where the street is, and where the window and counter are.
const GROUND: i32 = 30; // the far kerb: buildings stand on it, people walk on it
const ROAD_TOP: i32 = 33;
const GLASS_BOTTOM: i32 = 38;
const LEDGE: i32 = 40;
/// Window posts repeat this far apart, aligned to the core.
const PANE: i32 = 48;
/// Seats along the counter beyond the core come in bays this wide.
const SEATS: i32 = 40;
/// The speech bubbles' paper: a colour nothing else in the scene uses, so a
/// test can find who is talking.
pub(super) const BUBBLE: Rgb = rgb(0xfbf7ef);

const POST: Rgb = rgb(0x3a2418);
const POST_LIT: Rgb = rgb(0x5a3a26);
const LEDGE_TOP: Rgb = rgb(0x7a4e33);
const LEDGE_EDGE: Rgb = rgb(0x9a6a48);
const PANEL: Rgb = rgb(0x4a2f22);
const STEAM: Rgb = rgb(0xeee8de);

pub(super) fn cafe_street(c: &mut Canvas, frame: u64) {
    let (ox, oy) = cafe_origin(c);
    let (w, h) = (c.width as i32, c.height as i32);
    let ground = oy + GROUND;
    let road = oy + ROAD_TOP;
    let glass_bottom = oy + GLASS_BOTTOM;
    let ledge = oy + LEDGE;
    // The window takes all the height above the counter, under a ceiling
    // band of at most six rows: more screen is more street, not more room.
    let glass_top = oy.clamp(0, 6);
    c.origin(0, 0);

    // --- Outside. Sky first, the street in front of it. ---
    c.gradient(
        0,
        w,
        0,
        ground,
        &[rgb(0x0f1430), rgb(0x272552), rgb(0x45305e)],
    );
    // Buildings rise to between a little under the glass top and two fifths
    // of the way down, so a taller window shows taller buildings.
    let rise = ground - glass_top;
    street(
        c,
        frame,
        ox,
        (0, w),
        ground,
        (glass_top + 2, glass_top + 3 + rise * 2 / 5),
        5,
    );
    // Sidewalk, kerb, road.
    c.rect(0, ground, w, 2, rgb(0x3a3a4a));
    c.rect(0, ground, w, 1, rgb(0x4a4a5c));
    c.rect(0, ground + 2, w, 1, rgb(0x6a6a7a));
    c.gradient(0, w, road, glass_bottom, &[rgb(0x161624), rgb(0x1e1e2e)]);
    // The wet road mirrors the street, rippling.
    for y in road..glass_bottom {
        let depth = y - road;
        let source = ground - 2 - depth * 3;
        let ripple = ((depth as f64 * 1.7 + phase(frame, 2) * TAU).sin() * 1.2).round() as i32;
        for x in 0..w {
            let above = c.get(
                (x + ripple).clamp(0, w - 1) as usize,
                source.max(0) as usize,
            );
            let bright = above.iter().copied().max().unwrap_or(0);
            if bright > 0x90 {
                c.blend(x, y, above, 0.32 - depth as f64 * 0.04);
            }
        }
    }

    // People walking both ways along the far pavement.
    let walkers = (w / 34).max(3) as u64;
    for i in 0..walkers {
        let seed = hash(i + 4000);
        let cycles = 1 + seed % 2;
        let right = i % 2 == 0;
        let span = (w + 30) as f64;
        let t = phase_at(frame, cycles, unit(seed));
        let x = if right {
            t * span - 15.0
        } else {
            span - 15.0 - t * span
        };
        let step = phase_at(frame, 32, unit(seed.wrapping_add(1))) < 0.5;
        walker(c, x.round() as i32, ground + 1, look(seed), right, step);
    }

    // The tram wire, and a tram gliding past once a loop.
    let wire = oy + 15;
    c.rect(0, wire, w, 1, rgb(0x14141c));
    tram(c, frame, w, wire, road + 4);

    rain(
        c,
        frame,
        (0, 0, w, glass_bottom),
        (w * 3 / 4) as usize,
        rgb(0x9cc0f0),
        0.55,
        1,
    );

    // --- The window. ---
    // Breath on the glass, low down, and the café's warmth reflected in it.
    for y in road - 6..glass_bottom {
        let t = (y - (road - 6)) as f64 / 11.0;
        for x in 0..w {
            c.blend(x, y, rgb(0xd8c8b8), 0.05 + t * 0.07);
        }
    }
    // Drops running down the glass itself, slower and fatter than the rain.
    for i in 0..(w / 6).max(8) as u64 {
        let x = (hash(i + 900) % w as u64) as i32;
        let run = phase_at(frame, 1, unit(i + 901));
        let y = (oy as f64 + run * (GLASS_BOTTOM as f64 + 3.0)) as i32;
        for (k, a) in [(0, 0.7), (1, 0.4), (2, 0.25), (3, 0.12)] {
            if y - k < glass_bottom {
                c.blend(x, y - k, rgb(0xcfe0ff), a);
            }
        }
    }
    // The café's name painted on the upper pane, back to front from in here.
    lettering(c, ox + 14, glass_top + 1);

    // The frame: posts repeating along the canvas, a transom, the sill.
    let first = ox - PANE * ((ox + PANE - 1) / PANE + 1);
    let mut x = first;
    while x < w + PANE {
        c.rect(x - 1, 0, 3, glass_bottom, POST);
        c.rect(x - 1, 0, 1, glass_bottom, POST_LIT);
        x += PANE;
    }
    c.rect(0, glass_top + 7, w, 2, POST);
    c.rect(0, glass_top + 7, w, 1, POST_LIT);
    c.rect(0, glass_bottom, w, 2, POST);

    // The ceiling band, and lamps hanging from it in front of the glass.
    if glass_top > 0 {
        c.gradient(0, w, 0, glass_top, &[rgb(0x1c120e), rgb(0x2e1e16)]);
        c.rect(0, glass_top - 1, w, 1, POST);
        // At the posts, so they never hang in front of the lettering.
        let mut x = first;
        while x < w + PANE {
            pendant(c, frame, x, glass_top + 3);
            x += PANE;
        }
    }

    // --- Inside: the counter by the window. ---
    c.rect(0, ledge, w, h - ledge, PANEL);
    c.rect(0, ledge, w, 2, LEDGE_TOP);
    c.rect(0, ledge, w, 1, LEDGE_EDGE);
    c.rect(0, ledge + 2, w, 1, rgb(0x2e1c14));
    // Panelling, a brass foot rail, and floor tiles where there is room.
    let floor = (ledge + 14).max(h - (h - ledge) / 3);
    for x in 0..w {
        if (x - ox).rem_euclid(12) == 0 {
            c.rect(x, ledge + 3, 1, floor - ledge - 3, rgb(0x3a2418));
        }
    }
    if h - ledge > 12 {
        c.rect(0, ledge + 9, w, 1, rgb(0xd9a441));
        c.rect(0, ledge + 10, w, 1, rgb(0x8a6a2a));
    }
    if floor < h {
        for y in floor..h {
            for x in 0..w {
                let tile = ((x - ox).div_euclid(4) + (y - floor).div_euclid(3)) % 2 == 0;
                c.set(x, y, if tile { rgb(0x2a2420) } else { rgb(0x3a322a) });
            }
        }
        c.rect(0, floor, w, 1, rgb(0x1a1410));
    }
    // Somebody's umbrella, leaning on the counter, dripping.
    if h - ledge > 14 {
        umbrella(c, frame, ox + 88, ledge + 3, floor.min(h - 1));
    }

    // Things along the counter beyond the core, one per slot.
    let mut slot = (0 - ox).div_euclid(16) - 1;
    loop {
        let x = ox + slot * 16;
        if x > w {
            break;
        }
        if !(0..6).contains(&slot) {
            counter_thing(c, frame, x + 4, ledge, hash_at(slot, 77));
        }
        slot += 1;
    }

    // People on stools along the window beyond the core, in bays counted
    // out from its edges: pairs talking, someone on their phone, now and then
    // empty stools — but never next to the core, so a typical screen always
    // has company at the counter.
    let seat_floor = floor.min(h);
    let right_of_core = ox + WIDTH as i32;
    for side in [-1i32, 1] {
        let mut nth = 0;
        loop {
            let x = if side > 0 {
                right_of_core + nth * SEATS
            } else {
                ox - (nth + 1) * SEATS
            };
            if (side > 0 && x > w) || (side < 0 && x + SEATS < 0) {
                break;
            }
            let seed = hash_at(side * (nth + 1), 913);
            // Next to the core: a pair two times in three, else a phone.
            // Further out: pairs half the time, a phone a third, empty a sixth.
            let roll = if nth == 0 { seed % 3 * 2 } else { seed % 6 };
            if roll <= 2 {
                conversation(c, frame, (x + 6, x + 24), ledge, seat_floor, seed, true);
            } else if roll == 5 {
                stool(c, x + 8, ledge, seat_floor);
                stool(c, x + 24, ledge, seat_floor);
            } else {
                stool(c, x + 14, ledge, seat_floor);
                let glow = 0.6 + 0.4 * wave(frame, 2, unit(seed));
                let facing_right = (seed / 6).is_multiple_of(2);
                sitter(
                    c,
                    x + 14,
                    ledge,
                    look(seed),
                    facing_right,
                    Pose::Phone(glow),
                );
            }
            nth += 1;
        }
    }

    // The core's counter: a croissant, the latte, a notebook, a plant.
    c.origin(ox, oy);
    croissant(c, 10, LEDGE);
    latte(c, frame, 32, LEDGE);
    notebook(c, 50, LEDGE);
    c.rect(78, LEDGE - 4, 5, 4, rgb(0xd8cfc0));
    for (dx, dy) in [(1, -1), (2, -2), (3, -1), (0, -2), (4, -2), (2, -3)] {
        c.set(
            78 + dx,
            LEDGE - 4 + dy,
            if dy == -3 {
                rgb(0x6fb35f)
            } else {
                rgb(0x4f8a4b)
            },
        );
    }

    // A pair talking either side of the latte, their backs to the room.
    c.origin(0, 0);
    conversation(
        c,
        frame,
        (ox + 20, ox + 40),
        ledge,
        seat_floor,
        0x5eed,
        false,
    );
}

/// What a sitter is doing.
#[derive(Clone, Copy, PartialEq)]
enum Pose {
    /// Talking: mouth open or shut this frame, hand up or down.
    Talking { mouth: bool, gesture: bool },
    /// Listening, nodding now and then.
    Listening { nod: bool },
    /// Looking at a phone, its screen this bright.
    Phone(f64),
}

/// Two people on stools at `xs`, facing each other across the counter and
/// taking turns: four turns a loop, the speaker's bubble up for most of
/// theirs, their mouth and hand moving; the listener nodding.
fn conversation(
    c: &mut Canvas,
    frame: u64,
    (xa, xb): (i32, i32),
    ledge: i32,
    floor: i32,
    seed: u64,
    cups: bool,
) {
    let (a, b) = (look(seed), look(seed.wrapping_add(17)));
    stool(c, xa, ledge, floor);
    stool(c, xb, ledge, floor);
    // Where in the conversation we are: which turn, and how far into it.
    let at = phase_at(frame, 1, unit(seed)) * 4.0;
    let turn = at as u64;
    let into = at.fract();
    let a_talks = turn.is_multiple_of(2);
    let speaking = into < 0.75;
    let mouth = speaking && phase(frame, 20) < 0.5;
    let gesture = speaking && phase_at(frame, 5, unit(seed.wrapping_add(1))) < 0.4;
    let nod = phase_at(frame, 4, unit(seed.wrapping_add(2))) < 0.12;
    let talking = Pose::Talking { mouth, gesture };
    let listening = Pose::Listening { nod };
    sitter(
        c,
        xa,
        ledge,
        a,
        true,
        if a_talks { talking } else { listening },
    );
    sitter(
        c,
        xb,
        ledge,
        b,
        false,
        if a_talks { listening } else { talking },
    );
    // Their cups on the counter between them — unless they share one.
    if cups {
        c.rect(xa + 9, ledge - 3, 3, 3, rgb(0xf3eadc));
        c.rect(xb - 4, ledge - 3, 3, 3, rgb(0xf3eadc));
    }
    if speaking {
        let x = if a_talks { xa } else { xb };
        let kind = hash(seed.wrapping_add(turn)) % 6;
        bubble(c, frame, x + 3, ledge - 16, kind, a_talks);
    }
}

/// Someone sitting on a stool at the counter, side on, `x` their left edge,
/// facing right or left. Their head is over the glass, their back to us.
fn sitter(c: &mut Canvas, x: i32, ledge: i32, who: Look, facing_right: bool, pose: Pose) {
    let top = ledge - 9;
    let palette = [
        ('h', who.hair),
        ('f', who.skin),
        ('e', rgb(0x2a1d18)),
        ('c', who.coat),
        ('t', who.trousers),
        ('s', rgb(0x1a1a1a)),
    ];
    // Mirror a column for someone facing left.
    let fx = |dx: i32| if facing_right { x + dx } else { x + 7 - dx };
    let nod = matches!(pose, Pose::Listening { nod: true }) as i32;
    sprite(
        c,
        x,
        top + nod,
        &["..hhh...", ".hhhhf..", ".hhhef..", "..hfff..", "...ff..."],
        &palette,
        !facing_right,
    );
    sprite(
        c,
        x,
        top + 5,
        &[
            "..cccc..", ".cccccc.", ".cccccc.", ".ccccc..", ".ccccc..", ".ccccc..", ".cccccc.",
            ".tttttt.", ".....tt.", ".....tt.", ".....tt.", ".....tt.", ".....tt.", "....sss.",
        ],
        &palette,
        !facing_right,
    );
    let sleeve = mix(who.coat, rgb(0x000000), 0.2);
    let shoulder = (fx(4) as f64, (ledge - 3) as f64);
    match pose {
        Pose::Talking { mouth, gesture } => {
            if mouth {
                c.set(fx(5), top + 3, rgb(0x5a2a2a));
            }
            // The hand up mid-sentence, or resting on the counter.
            let hand = if gesture {
                (fx(7), ledge - 6)
            } else {
                (fx(7), ledge - 1)
            };
            c.line(shoulder, (hand.0 as f64, hand.1 as f64), sleeve);
            c.set(hand.0, hand.1, who.skin);
        }
        Pose::Listening { .. } => {
            c.line(shoulder, (fx(6) as f64, (ledge - 1) as f64), sleeve);
            c.set(fx(7), ledge - 1, who.skin);
        }
        Pose::Phone(glow) => {
            c.line(shoulder, (fx(6) as f64, (ledge - 5) as f64), sleeve);
            c.set(fx(7), ledge - 5, who.skin);
            let screen = mix(rgb(0x6fa8ff), rgb(0xe0f0ff), glow);
            c.rect(fx(7).min(fx(8)), ledge - 7, 1, 2, screen);
            c.glow(
                fx(6) as f64 + 0.5,
                (top + 2) as f64,
                4.0,
                rgb(0x9ac4ff),
                0.3 * glow,
            );
        }
    }
}

/// A bar stool: a round wooden seat on a pole, a foot ring, a base.
fn stool(c: &mut Canvas, x: i32, ledge: i32, floor: i32) {
    let seat = ledge + 4;
    c.rect(x, seat, 8, 1, rgb(0x7a4e33));
    c.rect(x + 1, seat + 1, 6, 1, rgb(0x4a2f22));
    c.rect(x + 3, seat + 2, 2, (floor - seat - 2).max(0), rgb(0x2a2a30));
    c.rect(x + 1, ledge + 10, 6, 1, rgb(0x8a8a9a));
    if floor - 1 > ledge + 10 {
        c.rect(x + 1, floor - 1, 6, 1, rgb(0x2a2a30));
    }
}

/// A speech bubble centred over `x`, its top at `y`: dots that come and go
/// as the sentence goes on, or now and then a heart or a note. The tail
/// points down toward the speaker.
fn bubble(c: &mut Canvas, frame: u64, x: i32, y: i32, kind: u64, tail_right: bool) {
    let (left, width, height) = (x - 4, 9, 6);
    c.rect(left + 1, y, width - 2, height, BUBBLE);
    c.rect(left, y + 1, width, height - 2, BUBBLE);
    let tail = if tail_right {
        left + 2
    } else {
        left + width - 3
    };
    c.set(tail, y + height, BUBBLE);
    let ink = rgb(0x4a4a5a);
    match kind {
        4 => {
            // A heart.
            for (dx, dy) in [
                (2, 1),
                (3, 1),
                (5, 1),
                (6, 1),
                (2, 2),
                (3, 2),
                (4, 2),
                (5, 2),
                (6, 2),
                (3, 3),
                (4, 3),
                (5, 3),
                (4, 4),
            ] {
                c.set(left + dx, y + dy, rgb(0xd1495b));
            }
        }
        5 => {
            // A note, as if humming the song.
            c.rect(left + 5, y + 1, 1, 3, ink);
            c.rect(left + 3, y + 3, 2, 2, ink);
            c.set(left + 6, y + 1, ink);
        }
        _ => {
            // Dots, one to three, as the sentence goes on.
            let dots = 1 + (phase(frame, 8) * 3.0) as i32;
            for k in 0..dots {
                c.set(left + 2 + k * 2, y + 3, ink);
            }
        }
    }
}

/// A tram crossing once a loop, from off one side to off the other, its
/// pantograph on the wire.
fn tram(c: &mut Canvas, frame: u64, w: i32, wire: i32, rail: i32) {
    let length = 46;
    let span = (w + length * 2 + 40) as f64;
    let x = (phase(frame, 1) * span) as i32 - length - 20;
    let (top, body) = (rail - 11, rgb(0xd8a23a));
    // Pantograph up to the wire.
    c.line(
        ((x + 22) as f64, (top - 1) as f64),
        ((x + 26) as f64, wire as f64),
        rgb(0x2a2a2a),
    );
    c.rect(x + 24, wire, 5, 1, rgb(0x2a2a2a));
    // Body, rounded at the ends, a stripe, lit windows with riders.
    c.rect(x + 1, top, length - 2, 10, body);
    c.rect(x, top + 1, length, 8, body);
    c.rect(x + 1, top, length - 2, 1, mix(body, rgb(0xffffff), 0.3));
    c.rect(x, top + 7, length, 1, rgb(0x8a3324));
    for k in 0..6 {
        let wx = x + 3 + k * 7;
        c.rect(wx, top + 2, 5, 4, rgb(0xffe2a0));
        if hash(k as u64 + 70).is_multiple_of(2) {
            let rider = look(hash(k as u64 + 71));
            c.rect(wx + 1, top + 4, 2, 2, rider.coat);
            c.rect(wx + 1, top + 3, 2, 1, rider.skin);
        }
    }
    // Headlight forward, its beam on the wet road.
    c.set(x + length - 1, top + 5, rgb(0xfff4c2));
    c.glow(
        (x + length + 3) as f64,
        (rail - 1) as f64,
        8.0,
        rgb(0xfff0b0),
        0.3,
    );
    c.rect(x + 4, rail - 1, 6, 1, rgb(0x1a1a1a));
    c.rect(x + length - 10, rail - 1, 6, 1, rgb(0x1a1a1a));
}

/// "CAFE" in gold paint, as it reads from inside: the word reversed and every
/// letter mirrored.
fn lettering(c: &mut Canvas, x: i32, y: i32) {
    const C: [&str; 5] = ["###", "#..", "#..", "#..", "###"];
    const A: [&str; 5] = [".#.", "#.#", "###", "#.#", "#.#"];
    const F: [&str; 5] = ["###", "#..", "##.", "#..", "#.."];
    const E: [&str; 5] = ["###", "#..", "##.", "#..", "###"];
    let paint = rgb(0xd9a441);
    // Seen from behind the glass: E, F, A, C, each flipped.
    for (k, letter) in [E, F, A, C].iter().enumerate() {
        for (dy, row) in letter.iter().enumerate() {
            for (dx, ch) in row.chars().rev().enumerate() {
                if ch == '#' {
                    c.blend(x + k as i32 * 5 + dx as i32, y + dy as i32, paint, 0.85);
                }
            }
        }
    }
}

/// The latte on its saucer, steam rising in two wisps.
fn latte(c: &mut Canvas, frame: u64, x: i32, ledge: i32) {
    c.rect(x - 2, ledge - 1, 11, 1, rgb(0xd8cfc0));
    c.rect(x, ledge - 6, 7, 5, rgb(0xf3eadc));
    c.rect(x, ledge - 6, 7, 1, rgb(0xc89a6a));
    c.set(x + 3, ledge - 6, rgb(0xf3eadc));
    c.rect(x + 7, ledge - 5, 1, 3, rgb(0xf3eadc));
    c.set(x + 8, ledge - 4, rgb(0xf3eadc));
    for wisp in 0..2u64 {
        for puff in 0..6u64 {
            let rise = phase_at(frame, 2, wisp as f64 / 2.0 + puff as f64 / 6.0);
            let px =
                x as f64 + 2.0 + wisp as f64 * 2.5 + ((rise * 1.4 + wisp as f64) * TAU).sin() * 1.2;
            let py = (ledge - 7) as f64 - rise * 10.0;
            c.blend(
                px.round() as i32,
                py.round() as i32,
                STEAM,
                (1.0 - rise) * 0.5,
            );
        }
    }
}

fn croissant(c: &mut Canvas, x: i32, ledge: i32) {
    c.rect(x - 1, ledge - 1, 12, 1, rgb(0xeee8de));
    super::parts::sprite(
        c,
        x,
        ledge - 5,
        &["...bbbb...", ".bBbbBbb..", "bbBbbbBbbb", ".bbbbbbbb."],
        &[('b', rgb(0xd08a3e)), ('B', rgb(0xa8642a))],
        false,
    );
}

fn notebook(c: &mut Canvas, x: i32, ledge: i32) {
    c.rect(x, ledge - 1, 18, 1, rgb(0xeee8de));
    c.rect(x + 8, ledge - 1, 2, 1, rgb(0xb0a898));
    for k in 0..3 {
        c.rect(x + 2, ledge - 1, 5, 1, rgb(0xeee8de));
        c.set(x + 2 + k * 2, ledge - 1, rgb(0x8a8a9a));
    }
    c.line(
        ((x + 12) as f64, (ledge - 2) as f64),
        ((x + 17) as f64, (ledge - 4) as f64),
        rgb(0xe9c46a),
    );
    c.set(x + 12, ledge - 2, rgb(0x2a2a2a));
}

/// Something on the counter, picked by `seed`: sugar, a vase, a candle,
/// napkins, a cup.
fn counter_thing(c: &mut Canvas, frame: u64, x: i32, ledge: i32, seed: u64) {
    match seed % 6 {
        0 => {
            // A sugar jar.
            c.rect(x, ledge - 5, 4, 5, rgb(0xcfe6ff));
            c.rect(x, ledge - 2, 4, 2, rgb(0xf3eadc));
            c.rect(x, ledge - 6, 4, 1, rgb(0x9aa0aa));
        }
        1 => {
            // A bud vase with a flower.
            c.rect(x + 1, ledge - 4, 2, 4, rgb(0x7ee0d6));
            c.rect(x + 2, ledge - 8, 1, 4, rgb(0x4f8a4b));
            let bloom = rgb([0xd1495b, 0xe9c46a, 0xff7eb6][(seed / 6 % 3) as usize]);
            c.rect(x + 1, ledge - 10, 3, 2, bloom);
        }
        2 => {
            // A candle in a jar, its flame flickering.
            c.rect(x, ledge - 4, 4, 4, rgb(0xb07a4a));
            c.rect(x + 1, ledge - 5, 2, 1, rgb(0xeee8de));
            let flick = wave(frame, 6, unit(seed));
            c.set(x + 1 + (flick > 0.5) as i32, ledge - 6, rgb(0xffd27a));
            c.glow(
                x as f64 + 2.0,
                (ledge - 6) as f64,
                5.0,
                rgb(0xffc86e),
                0.25 + 0.15 * flick,
            );
        }
        3 => {
            // A napkin holder.
            c.rect(x, ledge - 4, 5, 4, rgb(0x9aa0aa));
            c.rect(x + 1, ledge - 5, 3, 3, rgb(0xf3eadc));
        }
        4 => {
            // An empty cup on its saucer.
            c.rect(x - 1, ledge - 1, 7, 1, rgb(0xd8cfc0));
            c.rect(x, ledge - 4, 5, 3, rgb(0xf3eadc));
            c.set(x + 5, ledge - 3, rgb(0xf3eadc));
        }
        _ => {}
    }
}

/// A closed umbrella leaning against the counter front, its tip on the floor,
/// a drop falling from it into a small puddle once a loop.
fn umbrella(c: &mut Canvas, frame: u64, x: i32, top: i32, floor: i32) {
    let cloth = rgb(0x2f6b9a);
    let length = floor - top;
    for i in 0..length {
        let lean = i / 6;
        let width = if i < 3 {
            1
        } else if i < length - 3 {
            3
        } else {
            1
        };
        c.rect(x + lean - width / 2, top + i, width, 1, cloth);
    }
    c.rect(x - 1, top - 2, 2, 2, rgb(0x6b4632)); // the handle's crook
    c.set(x - 2, top - 1, rgb(0x6b4632));
    let tip = x + (length - 1) / 6;
    c.rect(tip - 3, floor, 8, 1, rgb(0x4a5a7a));
    let fall = phase(frame, 1);
    if fall < 0.5 {
        let dy = (fall / 0.5 * (floor - top - length / 2) as f64) as i32;
        c.set(tip + 1, top + length / 2 + dy, rgb(0x9cc0f0));
    }
}

/// A brass pendant lamp at column `x`, hanging from the ceiling to `bottom`.
fn pendant(c: &mut Canvas, frame: u64, x: i32, bottom: i32) {
    let sway = (phase_at(frame, 1, unit(x as u64)) * TAU).sin() * 0.5;
    c.line(
        (x as f64, 0.0),
        (x as f64 + sway, (bottom - 3) as f64),
        rgb(0x0e0a08),
    );
    let sx = (x as f64 + sway).round() as i32;
    c.glow(
        sx as f64 + 0.5,
        bottom as f64 + 1.0,
        12.0,
        rgb(0xffc86e),
        0.3,
    );
    c.rect(sx - 2, bottom - 3, 5, 1, rgb(0xb8862e));
    c.rect(sx - 3, bottom - 2, 7, 2, rgb(0xd9a441));
    c.rect(sx - 1, bottom, 3, 1, rgb(0xfff1c2));
}
