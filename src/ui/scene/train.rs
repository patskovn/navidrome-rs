//! The night train: a carriage at night, passengers on the bench, and the
//! country going by outside in layers — hills, trees, poles, the embankment —
//! each scrolling at its own whole-pixel speed, nearer ones faster.

use std::f64::consts::TAU;

use super::parts::{look, scrolled, sprite};
use super::*;

/// The scene's anchor. Its subject is the passengers on the bench, whose
/// seat front is the core's row 44.
pub(super) fn train_origin(c: &Canvas) -> (i32, i32) {
    raised_origin(c, 44, 6)
}

/// Core rows, from the bench down.
const WINDOW_BOTTOM: i32 = 31;
const BENCH: i32 = 38; // the seat's top
const SEAT_FRONT: i32 = 44;
/// Windows repeat this far apart, aligned to the core.
const BAY: i32 = 40;
const WINDOW: i32 = 32;

const WALL: Rgb = rgb(0x3a405e);
const CEILING: Rgb = rgb(0x23263a);
const STEEL: Rgb = rgb(0xb8bccb);
const STEEL_DARK: Rgb = rgb(0x6e7286);
const CUSHION: Rgb = rgb(0x8a3a4a);
const CUSHION_LIT: Rgb = rgb(0xa8505e);

pub(super) fn night_train(c: &mut Canvas, frame: u64) {
    let (ox, oy) = train_origin(c);
    let (w, h) = (c.width as i32, c.height as i32);
    // Each rail joint bounces the passengers on their seats: four times a
    // loop. Only them — a whole carriage jumping a pixel reads as a glitch.
    let jolt = (phase(frame, 4) < 0.06) as i32;

    // Windows grow upward with the screen; above them the grab rail, the
    // luggage rack and the ceiling stack up as room allows.
    let bottom = oy + WINDOW_BOTTOM;
    let top = oy + 11 - (oy * 2 / 3).clamp(0, 28);
    let rail = top - 3;
    let rack = top - 6;
    // The light strip runs along the top when there is room for the adverts
    // under it; otherwise just above the rack.
    let light = if rack >= 12 { 2 } else { (rack - 5).max(1) };
    let first = ox - BAY * ((ox + BAY - 1) / BAY + 1);

    c.origin(0, 0);
    outside(c, frame, w, ox, (top, bottom));

    // --- The carriage, drawn over the view with the windows left open. ---
    let in_window = |x: i32, y: i32| -> bool {
        if y < top || y >= bottom {
            return false;
        }
        let along = (x - first).rem_euclid(BAY);
        let (left, right) = (4, 4 + WINDOW);
        if along < left || along >= right {
            return false;
        }
        let corner_x = along == left || along == right - 1;
        let corner_y = y == top || y == bottom - 1;
        !(corner_x && corner_y)
    };
    for y in 0..h {
        for x in 0..w {
            if !in_window(x, y) {
                c.set(x, y, if y < rack { CEILING } else { WALL });
            } else if (x - first + y).rem_euclid(29) < 2 {
                // A faint reflection of the carriage lights on the glass.
                c.blend(x, y, rgb(0xffffff), 0.07);
            }
        }
    }
    let mut bay = first;
    while bay < w + BAY {
        c.rect(bay + 3, top - 1, WINDOW + 2, 1, STEEL_DARK);
        c.rect(bay + 3, bottom, WINDOW + 2, 1, STEEL);
        bay += BAY;
    }

    // Ceiling: a light strip, adverts if there is room under it.
    c.rect(0, light, w, 1, rgb(0xfff1c2));
    for dy in 1..4 {
        for x in 0..w {
            c.blend(x, light + dy, rgb(0xfff1c2), 0.12 / dy as f64);
        }
    }
    // Adverts under the light, clear of the bags when there is room.
    if rack - light >= 14 {
        adverts(c, first, w, rack - 12);
    } else if rack - light >= 11 {
        adverts(c, first, w, rack - 9);
    }
    // The luggage rack, bags on it.
    c.rect(0, rack, w, 1, STEEL);
    c.rect(0, rack + 2, w, 1, STEEL_DARK);
    let mut bay = first;
    while bay < w + BAY {
        for k in 0..2 {
            let seed = hash_at((bay - ox) / BAY * 2 + k, 31);
            if !seed.is_multiple_of(3) {
                bag(c, bay + 6 + k * 16, rack, seed);
            }
        }
        bay += BAY;
    }
    // The grab rail, and a few straps swinging from it.
    c.rect(0, rail, w, 1, STEEL);
    let mut x = first + 10;
    while x < w + BAY {
        let swing = (phase_at(frame, 2, unit((x - first) as u64 % 97)) * TAU).sin();
        let tip = x as f64 + swing * 1.2;
        c.line(
            (x as f64, rail as f64),
            (tip, (rail + 3) as f64),
            rgb(0x2a2a2a),
        );
        c.rect(tip.round() as i32 - 1, rail + 4, 2, 2, STEEL_DARK);
        x += 20;
    }

    // Bench and floor.
    let bench = oy + BENCH;
    let seat_front = oy + SEAT_FRONT;
    c.rect(0, bench, w, 2, CUSHION_LIT);
    c.rect(0, bench + 2, w, seat_front - bench - 2, CUSHION);
    for x in 0..w {
        if (x - first).rem_euclid(13) == 1 {
            c.rect(x, bench, 1, seat_front - bench, rgb(0x5a2430));
        }
    }
    c.rect(0, seat_front, w, 2, rgb(0x1a1c2a));
    let floor = seat_front + 2;
    c.gradient(0, w, floor, h, &[rgb(0x2e3044), rgb(0x3a3c52)]);
    // Window light falls on the floor below each window; a yellow line
    // keeps feet back from the doors.
    for y in floor..h {
        for x in 0..w {
            let along = (x - first).rem_euclid(BAY);
            if (6..4 + WINDOW).contains(&along) {
                c.blend(
                    x,
                    y,
                    rgb(0x5a6a9a),
                    0.18 * (1.0 - (y - floor) as f64 / (h - floor).max(1) as f64),
                );
            }
        }
    }
    // Rubber ribbing, and the bench faintly mirrored in the floor's sheen.
    for y in (floor..h).step_by(2) {
        for x in 0..w {
            c.blend(x, y, rgb(0x4a4c64), 0.25);
        }
    }
    for dy in 0..4 {
        for x in 0..w {
            c.blend(x, floor + 1 + dy, CUSHION, 0.12 - dy as f64 * 0.025);
        }
    }
    if h - floor > 6 {
        c.rect(0, floor + 4, w, 1, rgb(0xd9b441));
    }
    // People on the bench, legs down its front, and their things at their feet.
    let mut seat = first;
    while seat < w + BAY {
        for k in 0..3 {
            let sx = seat + 3 + k * 13;
            let seed = hash_at((seat - ox) / BAY * 3 + k, 53);
            passenger(c, frame, sx, bench - jolt, seed);
            if h - floor > 3 {
                floor_thing(c, sx + 9, floor + 2, seed);
            }
        }
        seat += BAY;
    }
    // Poles, floor to rail, between the bays.
    let mut pole = first + BAY - 1;
    while pole < w + BAY {
        c.rect(pole, rail, 2, h - rail, STEEL);
        c.rect(pole, rail, 1, h - rail, rgb(0xe8eaf2));
        pole += BAY;
    }
}

/// The view out of the windows, rows `top..bottom`: sky, a moon, and the
/// country in layers, each nearer layer scrolling faster.
fn outside(c: &mut Canvas, frame: u64, w: i32, ox: i32, (top, bottom): (i32, i32)) {
    let horizon = bottom - 5;
    c.gradient(
        0,
        w,
        0,
        horizon,
        &[rgb(0x070b1e), rgb(0x101a3a), rgb(0x2a3a6a)],
    );
    // Stars, and a moon too far away to move.
    let sky = (horizon - 6 - top).max(1);
    for i in 0..(w as u64 * sky as u64 / 40) {
        let x = (hash(i + 600) % w as u64) as i32;
        let y = top + (hash(i + 601) % sky as u64) as i32;
        let twinkle = wave(frame, 1 + hash(i + 602) % 3, unit(i + 603));
        c.blend(x, y, rgb(0xffffff), 0.2 + 0.6 * twinkle);
    }
    let (mx, my) = ((ox + 66) as f64, (top + 3 + sky / 4) as f64);
    c.glow(mx, my, 10.0, rgb(0xf3eac2), 0.18);
    c.disc(mx, my, 3.2, rgb(0xf3eac2));

    // Far hills, a village light here and there: 1 px a frame.
    for x in 0..w {
        let u = scrolled(x, 160, frame, 1);
        let t = u as f64 / 160.0 * TAU;
        let height = 6.0 + (t * 2.0).sin() * 2.5 + (t * 5.0 + 1.0).sin() * 1.5;
        let ridge = horizon - height.round() as i32;
        c.rect(x, ridge, 1, horizon + 6 - ridge, rgb(0x1e2a58));
        c.set(x, ridge, rgb(0x2e3c72));
        if hash(u as u64 + 1700).is_multiple_of(7) {
            c.set(x, ridge + 2 + (u % 3), rgb(0xffcf7a));
        }
    }
    // Trees and the odd lit house: 2 px a frame.
    for x in 0..w {
        let u = scrolled(x, 80, frame, 4);
        let seed = hash(u as u64 / 6 + 2900);
        let crown = horizon - (2 + (seed % 5) as i32);
        let wobble = (u % 6 - 3).abs() / 2;
        c.rect(
            x,
            crown + wobble,
            1,
            horizon + 6 - crown - wobble,
            rgb(0x0e1530),
        );
        if seed.is_multiple_of(5) && u % 6 == 3 {
            c.set(x, horizon - 1, rgb(0xffd98a));
        }
    }
    // Telephone poles and their wires: 3 px a frame.
    let wire = top + (horizon - top) / 3;
    for x in 0..w {
        let u = scrolled(x, 48, frame, 10);
        let sag = (1.0 - ((u as f64 / 48.0) * 2.0 - 1.0).powi(2)) * 3.0;
        c.set(x, wire + sag.round() as i32, rgb(0x070912));
        c.set(x, wire + 3 + sag.round() as i32, rgb(0x070912));
        if u < 2 {
            c.rect(x, wire - 2, 1, horizon + 6 - wire, rgb(0x070912));
        }
        if u == 0 {
            c.rect(x - 2, wire - 1, 6, 1, rgb(0x070912));
        }
    }
    // The embankment rushing past right under the window: 4 px a frame.
    for x in 0..w {
        let u = scrolled(x, 32, frame, 20);
        let bump = (hash(u as u64 / 2 + 3100) % 3) as i32;
        c.rect(
            x,
            horizon + 2 - bump,
            1,
            bottom - horizon + bump,
            rgb(0x080b18),
        );
    }
}

/// Small framed adverts above the windows, one per bay, their colours and
/// layout picked by position.
fn adverts(c: &mut Canvas, first: i32, w: i32, y: i32) {
    let mut bay = first;
    while bay < w + BAY {
        let seed = hash_at(bay - first, 61);
        let background =
            rgb([0xe9c46a, 0x7ee0d6, 0xff7eb6, 0xeee8de, 0x9ab0ff][(seed % 5) as usize]);
        let ink = rgb([0x2a2a3a, 0x8a3324, 0x2f4858][(seed / 5 % 3) as usize]);
        c.rect(bay + 6, y, 28, 7, rgb(0x1a1c2a));
        c.rect(bay + 7, y + 1, 26, 5, background);
        c.rect(bay + 9, y + 2, 10, 1, ink);
        c.rect(bay + 9, y + 4, 14, 1, mix(ink, background, 0.5));
        c.disc((bay + 28) as f64, (y + 3) as f64 + 0.5, 2.0, ink);
        bay += BAY;
    }
}

/// A bag on the luggage rack.
fn bag(c: &mut Canvas, x: i32, rack: i32, seed: u64) {
    let color = rgb([0x4f6b4a, 0x7a3b45, 0xc79a3b, 0x2f4858, 0x6b4632][(seed / 3 % 5) as usize]);
    let width = 6 + (seed / 17 % 6) as i32;
    let height = 3 + (seed / 41 % 2) as i32;
    c.rect(x, rack - height, width, height, color);
    c.rect(x, rack - height, width, 1, mix(color, rgb(0xffffff), 0.25));
    c.rect(
        x + width / 2 - 1,
        rack - height - 1,
        3,
        1,
        mix(color, rgb(0x000000), 0.4),
    );
}

/// Something at a passenger's feet: a backpack, a shopping bag, a folded
/// umbrella — or nothing.
fn floor_thing(c: &mut Canvas, x: i32, floor: i32, seed: u64) {
    match seed / 7 % 6 {
        0 => {
            let pack = rgb([0x4f6b4a, 0xc79a3b, 0x2f6b9a][(seed / 3 % 3) as usize]);
            c.rect(x, floor - 5, 5, 6, pack);
            c.rect(x, floor - 5, 5, 1, mix(pack, rgb(0xffffff), 0.25));
            c.rect(x + 1, floor - 2, 3, 2, mix(pack, rgb(0x000000), 0.3));
        }
        1 => {
            c.rect(x, floor - 4, 4, 5, rgb(0xeee8de));
            c.rect(x + 1, floor - 6, 2, 2, rgb(0xb0a898));
            c.set(x + 1, floor - 2, rgb(0x6fb35f));
        }
        2 => {
            c.line(
                (x as f64, (floor - 6) as f64),
                ((x + 2) as f64, floor as f64),
                rgb(0x2a2a2a),
            );
            c.set(x - 1, floor - 6, rgb(0x6b4632));
        }
        _ => {}
    }
}

/// Someone on the bench, seen from the front, or an empty seat. `seed` picks
/// who, and what they are doing: dozing, reading, on a phone, listening.
fn passenger(c: &mut Canvas, frame: u64, x: i32, bench: i32, seed: u64) {
    let what = seed % 5;
    if what == 0 {
        return;
    }
    let who = look(seed);
    let doze = what == 1;
    // A doze nods: the head drops a row for a while, then lifts.
    let nod = doze && phase_at(frame, 1, unit(seed)) > 0.55;
    let lean = doze as i32;
    let head_y = bench - 15 + nod as i32;
    sprite(
        c,
        x + 2 + lean,
        head_y,
        &[".hhh.", "hhhhh", "hfffh", ".fff."],
        &[('h', who.hair), ('f', who.skin)],
        false,
    );
    let eye = if doze {
        mix(who.skin, rgb(0x000000), 0.4)
    } else {
        rgb(0x2a1d18)
    };
    c.set(x + 3 + lean, head_y + 2, eye);
    c.set(x + 5 + lean, head_y + 2, eye);
    // Shoulders and body, down to the seat; knees over its edge.
    sprite(
        c,
        x,
        bench - 11,
        &[
            "..ccccc..",
            ".ccccccc.",
            "ccccccccc",
            "ccccccccc",
            "ccccccccc",
            "ccccccccc",
            ".ccccccc.",
        ],
        &[('c', who.coat)],
        false,
    );
    c.rect(x + 1, bench - 4, 7, 4, who.trousers);
    match what {
        2 => {
            // Reading: a book in both hands; a page turns once a loop.
            c.rect(x + 1, bench - 8, 7, 4, rgb(0xc8402a));
            c.rect(x + 2, bench - 8, 5, 3, rgb(0xeee8de));
            if phase_at(frame, 1, unit(seed + 5)) < 0.08 {
                c.rect(x + 4, bench - 9, 2, 4, rgb(0xffffff));
            }
            c.set(x + 1, bench - 6, who.skin);
            c.set(x + 7, bench - 6, who.skin);
        }
        3 => {
            // On a phone: its glow on their face, brightening and dimming.
            let glow = 0.5 + 0.5 * wave(frame, 2, unit(seed + 6));
            c.rect(
                x + 3,
                bench - 8,
                3,
                2,
                mix(rgb(0x6fa8ff), rgb(0xd8ecff), glow),
            );
            c.glow(
                x as f64 + 4.5,
                (bench - 9) as f64,
                4.0,
                rgb(0x9ac4ff),
                0.25 * glow,
            );
            c.set(x + 2, bench - 7, who.skin);
            c.set(x + 6, bench - 7, who.skin);
        }
        4 => {
            // Headphones on.
            c.rect(x + 1, head_y - 1, 7, 1, rgb(0x2a2a2a));
            c.rect(x + 1, head_y + 1, 1, 3, rgb(0x2a2a2a));
            c.rect(x + 7, head_y + 1, 1, 3, rgb(0x2a2a2a));
        }
        _ => {
            // Dozing: hands folded in the lap.
            c.rect(x + 3, bench - 5, 3, 1, who.skin);
        }
    }
    // Shins down the seat front, shoes on the floor.
    c.rect(x + 2, bench, 2, 6, who.trousers);
    c.rect(x + 5, bench, 2, 6, who.trousers);
    c.rect(x + 2, bench + 6, 2, 1, rgb(0x1a1a1a));
    c.rect(x + 5, bench + 6, 2, 1, rgb(0x1a1a1a));
}
