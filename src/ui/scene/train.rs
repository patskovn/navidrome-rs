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
/// The band on the conductor's cap: a colour nothing else uses, so a test
/// can follow them down the carriage.
pub(super) const CAP_BAND: Rgb = rgb(0xe0b040);
/// The silhouettes of the people on our side of the carriage.
pub(super) const SHADOW: Rgb = rgb(0x0b0d16);

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
    // Lamps going by outside throw light across the floor: slanted patches
    // sliding the way the country does, 3 px a frame, whole turns a loop.
    for y in floor..h {
        for x in 0..w {
            let u = scrolled(x + (y - floor), 120, frame, 4);
            if u < 14 {
                let peak = 1.0 - (u as f64 - 7.0).abs() / 7.0;
                c.blend(x, y, rgb(0xffe8b0), 0.17 * peak);
            }
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

    // Our side of the carriage, along the bottom edge, when there is floor
    // enough to look across: it takes the last rows.
    let ours = h - floor >= 14;
    let aisle_end = if ours { h - 10 } else { h - 1 };

    // The conductor, working down the carriage just in front of the bench.
    conductor(c, frame, w, (floor + 9).min(h - 1));

    // People standing in the aisle, as many rows as there is room for,
    // farther rows first, swaying at each rail joint.
    let (back, front) = (floor + 15, aisle_end);
    if front >= back {
        let rows = (front - back) / 14 + 1;
        for r in 0..rows {
            let feet = if rows == 1 {
                front
            } else {
                back + (front - back) * r / (rows - 1)
            };
            let stagger = if r % 2 == 0 { 0 } else { 11 };
            let mut x = first + stagger;
            while x < w + 22 {
                let seed = hash_at((x - ox).div_euclid(11) * 7 + r, 211);
                if !seed.is_multiple_of(3) {
                    rider(c, frame, x, feet, seed, jolt);
                }
                x += 22;
            }
        }
    }

    if ours {
        our_side(c, frame, ox, w, h);
    }
}

/// Someone standing in the aisle, side on, `x` their left edge, feet on
/// `feet`. Most hold a strap; others have a suitcase, a bike, a dog at their
/// feet, or a phone. At each rail joint they sway a pixel.
fn rider(c: &mut Canvas, frame: u64, x: i32, feet: i32, seed: u64, jolt: i32) {
    let who = look(seed);
    let right = (seed / 3).is_multiple_of(2);
    let fx = |dx: i32| if right { x + dx } else { x + 6 - dx };
    let palette = [
        ('h', who.hair),
        ('f', who.skin),
        ('e', rgb(0x2a1d18)),
        ('c', who.coat),
        ('t', who.trousers),
        ('s', rgb(0x1a1a1a)),
    ];
    let top = feet - 15;
    let sway = jolt * if right { 1 } else { -1 };
    let what = seed / 7 % 6;
    // Things beside them go behind them.
    match what {
        2 => {
            // A suitcase on wheels.
            let sx = fx(8).min(fx(14));
            c.rect(sx, feet - 7, 7, 7, rgb(0x2f6b9a));
            c.rect(sx, feet - 7, 7, 1, rgb(0x4a86b8));
            c.rect(sx + 3, feet - 10, 1, 3, rgb(0x8a8a9a));
            c.rect(sx + 2, feet - 10, 3, 1, rgb(0x8a8a9a));
        }
        3 => {
            // A bike, held by the saddle.
            let (b0, b1) = (fx(8) as f64, fx(19) as f64);
            let (lo, hi) = (b0.min(b1), b0.max(b1));
            for wx in [lo, hi] {
                for k in 0..16 {
                    let a = k as f64 / 16.0 * TAU;
                    c.set(
                        (wx + a.cos() * 3.0).round() as i32,
                        (feet as f64 - 3.0 + a.sin() * 3.0).round() as i32,
                        rgb(0x1a1a22),
                    );
                }
            }
            let frame_color = rgb([0xd1495b, 0x3fa39a, 0xe9c46a][(seed / 5 % 3) as usize]);
            c.line(
                (lo, feet as f64 - 3.0),
                (lo + 5.0, feet as f64 - 8.0),
                frame_color,
            );
            c.line(
                (lo + 5.0, feet as f64 - 8.0),
                (hi, feet as f64 - 3.0),
                frame_color,
            );
            c.line(
                (lo + 5.0, feet as f64 - 8.0),
                (lo + 6.0, feet as f64 - 3.0),
                frame_color,
            );
            c.rect(lo as i32 + 3, feet - 10, 3, 1, rgb(0x2a2a2a));
        }
        4 => {
            // A dog lying at their feet, its tail going.
            let dx = fx(7).min(fx(15));
            let fur = rgb([0x8a5a3c, 0x2a2a2a, 0xd9a441][(seed / 5 % 3) as usize]);
            c.ellipse(dx as f64 + 4.5, feet as f64 - 1.5, 4.5, 1.8, fur);
            c.ellipse(
                dx as f64 + if right { 8.5 } else { 0.5 },
                feet as f64 - 2.5,
                1.8,
                1.6,
                fur,
            );
            let wag = (phase_at(frame, 6, unit(seed)) < 0.5) as i32;
            let tail = if right { dx - 1 } else { dx + 9 };
            c.set(tail, feet - 2 - wag, fur);
        }
        _ => {}
    }
    sprite(
        c,
        x + sway,
        top,
        &["..hhh..", ".hhhhf.", ".hhhef.", "..hff..", "...f..."],
        &palette,
        !right,
    );
    sprite(
        c,
        x,
        top + 5,
        &[
            "..ccc..", ".ccccc.", ".ccccc.", ".ccccc.", ".cccc..", ".cccc..", "..tt...", "..tt...",
            "..tt...", "..tt...", "..ss...",
        ],
        &palette,
        !right,
    );
    let sleeve = mix(who.coat, rgb(0x000000), 0.2);
    let shoulder = (fx(3) as f64, (top + 6) as f64);
    match what {
        0 | 1 => {
            // An arm up to a strap hanging from above.
            let hand = (fx(4) + sway, top - 4);
            c.line(shoulder, (hand.0 as f64, hand.1 as f64 + 1.0), sleeve);
            c.set(hand.0, hand.1, who.skin);
            c.line(
                (hand.0 as f64, (top - 9) as f64),
                (hand.0 as f64, (hand.1 - 1) as f64),
                rgb(0x2a2a2a),
            );
            c.rect(hand.0 - 1, hand.1 - 1, 3, 1, STEEL_DARK);
        }
        5 => {
            // A phone in hand, lighting the face.
            let glow = 0.5 + 0.5 * wave(frame, 2, unit(seed));
            c.line(shoulder, (fx(5) as f64, (top + 5) as f64), sleeve);
            c.set(fx(5), top + 4, mix(rgb(0x6fa8ff), rgb(0xe0f0ff), glow));
            c.glow(
                fx(4) as f64 + 0.5,
                (top + 3) as f64,
                3.5,
                rgb(0x9ac4ff),
                0.25 * glow,
            );
        }
        _ => {
            // A hand on the suitcase, the bike, the lead.
            c.line(shoulder, (fx(6) as f64, (top + 9) as f64), sleeve);
            c.set(fx(7), top + 9, who.skin);
        }
    }
}

/// The conductor: navy uniform, a cap with a gold band, working down the
/// carriage once a loop. Four times they stop to check a ticket, arm out to
/// the bench, then walk on.
fn conductor(c: &mut Canvas, frame: u64, w: i32, feet: i32) {
    let t = phase(frame, 1) * 4.0;
    let (leg, into) = (t.floor(), t.fract());
    let walking = into >= 0.25;
    let along = (leg + if walking { (into - 0.25) / 0.75 } else { 0.0 }) / 4.0;
    let x = (along * (w + 24) as f64) as i32 - 12;
    let step = walking && phase(frame, 32) < 0.5;
    let navy = rgb(0x1f2a48);
    let legs: [&str; 2] = if step {
        ["..n.n..", ".n...n."]
    } else {
        ["..nn...", "..nn..."]
    };
    let rows = [
        "..kkk..", ".kgggk.", "..hff..", "..fef..", "...f...", "..nnn..", ".nnynn.", ".nnnnn.",
        ".nnynn.", ".nnnn..", ".nnnn..", legs[0], legs[0], legs[1], "..ss...",
    ];
    sprite(
        c,
        x,
        feet - 14,
        &rows,
        &[
            ('k', navy),
            ('g', CAP_BAND),
            ('h', rgb(0x3a2a1a)),
            ('f', rgb(0xd9a07a)),
            ('e', rgb(0x2a1d18)),
            ('n', navy),
            ('y', rgb(0xe9c46a)),
            ('s', rgb(0x1a1a1a)),
        ],
        false,
    );
    if !walking {
        // Arm out toward the bench, holding the punch.
        c.line(
            ((x + 4) as f64, (feet - 9) as f64),
            ((x + 7) as f64, (feet - 12) as f64),
            navy,
        );
        c.rect(x + 7, feet - 13, 2, 2, rgb(0xb8bccb));
    }
}

/// Our side of the carriage: heads and shoulders from behind along the
/// bottom edge, dark against the floor, rim-lit by the ceiling lights. Now
/// and then one turns, a sliver of face catching the light.
fn our_side(c: &mut Canvas, frame: u64, ox: i32, w: i32, h: i32) {
    let rim = rgb(0xffe8b0);
    let mut slot = (0 - ox).div_euclid(18) - 1;
    loop {
        let x = ox + slot * 18;
        if x > w {
            break;
        }
        let seed = hash_at(slot, 77);
        if !seed.is_multiple_of(4) {
            let cx = x as f64 + 9.0 + (seed % 5) as f64 - 2.0;
            let head = (h - 7) as f64 + (seed / 5 % 2) as f64;
            let hood = (seed / 11).is_multiple_of(4);
            c.ellipse(cx, h as f64 + 2.0, 9.0, 5.0, SHADOW);
            c.ellipse(cx, head, if hood { 4.4 } else { 3.6 }, 4.2, SHADOW);
            // The ceiling lights catch the crown of each head, and the
            // shoulders' tops more faintly.
            let rx = if hood { 4.4 } else { 3.6 };
            for dx in -2i32..=2 {
                let t = dx as f64 / rx;
                let top = head - 4.2 * (1.0 - t * t).max(0.0).sqrt();
                c.blend(
                    cx as i32 + dx,
                    top.round() as i32,
                    rim,
                    0.4 - dx.abs() as f64 * 0.08,
                );
            }
            for dx in [-7i32, -6, 6, 7] {
                let t = dx as f64 / 9.0;
                let top = h as f64 + 2.0 - 5.0 * (1.0 - t * t).max(0.0).sqrt();
                c.blend(cx as i32 + dx, top.round() as i32, rim, 0.2);
            }
            // Turning to look out, now and then.
            let turn = phase_at(frame, 1, unit(seed));
            if (0.3..0.42).contains(&turn) {
                let side = if seed.is_multiple_of(2) { 3 } else { -3 };
                c.set(cx as i32 + side, head as i32, rgb(0x6a4a3a));
                c.set(cx as i32 + side, head as i32 + 1, rgb(0x5a3a2a));
            }
        }
        slot += 1;
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
