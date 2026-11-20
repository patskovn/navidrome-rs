//! Night study: somebody at their desk, seen from behind, writing and nodding
//! along, a big window full of falling snow over the city, a goldfish going
//! round its bowl — and the room built out around them to fill the screen.
//!
//! An original character, not anyone's mascot: back to us, in a mustard
//! beanie and a teal hoodie, in a teal room with snow outside.

use std::f64::consts::TAU;

use super::parts::sprite;
use super::*;

/// The scene's anchor. Its subject is the desk with the writer at it; the
/// desktop is the core's row 38.
pub(super) fn study_origin(c: &Canvas) -> (i32, i32) {
    raised_origin(c, 38, 4)
}

const DESK: i32 = 38;

const WALL: Rgb = rgb(0x1c3340);
const WALL_LOW: Rgb = rgb(0x24404a);
const WOOD: Rgb = rgb(0x6b4632);
const WOOD_EDGE: Rgb = rgb(0x80553d);
const WARM: Rgb = rgb(0xffc86e);
const SNOW: Rgb = rgb(0xf4f8ff);

pub(super) fn night_study(c: &mut Canvas, frame: u64) {
    let (ox, oy) = study_origin(c);
    let (w, h) = (c.width as i32, c.height as i32);
    let desk = oy + DESK;
    c.origin(0, 0);

    // The room: deep teal walls, a stripe of wallpaper pattern.
    c.gradient(0, w, 0, desk, &[rgb(0x13232e), WALL, WALL_LOW]);
    for x in 0..w {
        if (x - ox).rem_euclid(8) == 4 {
            for y in 0..desk {
                if (y - oy).rem_euclid(4) == 0 {
                    c.blend(x, y, rgb(0x3a6a72), 0.35);
                }
            }
        }
    }
    // Shelves and pictures beside the window, borrowed from the desk.
    let left = ox;
    let right = w - (ox + WIDTH as i32);
    if left >= 12 {
        super::desk::wall_shelves(c, frame, 2, left - 3, oy + 2);
    }
    if right >= 24 {
        super::desk::clock(c, frame, ox + WIDTH as i32 + 11, oy + 10);
    }
    if right >= 44 {
        super::desk::poster(c, ox + WIDTH as i32 + 25, oy + 6);
    }
    if oy >= 6 {
        super::desk::fairy_lights(c, frame, 0, w, oy - 4);
    }

    // On a tall wall, a small gallery over the window and a plant hanging
    // beside it.
    if oy >= 30 {
        super::desk::poster(c, ox + 30, oy - 28);
        super::desk::leaf_print(c, ox + 50, oy - 24);
        super::desk::poster_small(c, ox + 14, oy - 22);
    }
    if oy >= 16 {
        super::desk::hanging_plant(c, frame, ox + 90, 0, oy + 2);
    }

    c.origin(ox, oy);
    window(c, frame);

    // The desk across the canvas, and the room under it.
    c.origin(0, 0);
    c.rect(0, desk, w, h - desk, rgb(0x4a2f22));
    c.rect(0, desk, w, 3, WOOD);
    c.rect(0, desk, w, 1, WOOD_EDGE);
    super::desk::under_desk(c, ox, desk, w, h, false);
    // A backpack slumped under the desk, if there is room for it.
    if h - desk > 22 {
        backpack(c, ox + 58, h - (h - desk) * 2 / 5);
    }

    c.origin(ox, oy);
    desk_things(c, frame);
    writer(c, frame, 40, DESK);
}

/// A mustard backpack sitting on the floor, `floor` its base row.
fn backpack(c: &mut Canvas, x: i32, floor: i32) {
    let body = rgb(0xd9a441);
    let shade = rgb(0xb8862e);
    c.rect(x + 1, floor - 9, 9, 9, body);
    c.rect(x, floor - 7, 11, 7, body);
    c.rect(x + 1, floor - 9, 9, 1, rgb(0xf0c66a));
    c.rect(x + 2, floor - 5, 7, 4, shade);
    c.rect(x + 2, floor - 5, 7, 1, rgb(0x8a6a2a));
    c.rect(x + 4, floor - 11, 3, 2, rgb(0x6b4a1e));
    c.set(x + 5, floor - 4, rgb(0xd8d0c8));
}

/// The big window: a night city under falling snow, snow banking on the
/// sill, the frame dark wood.
fn window(c: &mut Canvas, frame: u64) {
    let (x0, y0, x1, y1) = (14, 3, 82, 33);
    c.rect(x0 - 2, y0 - 2, x1 - x0 + 4, y1 - y0 + 4, rgb(0x3a2418));
    c.gradient(
        x0,
        x1,
        y0,
        y1,
        &[rgb(0x0a1426), rgb(0x16284a), rgb(0x2a3c64)],
    );
    // The moon behind thin cloud.
    c.glow(70.0, 9.0, 8.0, rgb(0xe8f0ff), 0.25);
    c.disc(70.0, 9.0, 2.6, rgb(0xeef4ff));
    // The city: a far row and a near row of buildings, windows lit.
    for (row, base, color, salt) in [
        (0, y1 - 4, rgb(0x1e2c4c), 11u64),
        (1, y1, rgb(0x121c34), 23),
    ] {
        let mut x = x0;
        while x < x1 {
            let s = hash(x as u64 * 7 + salt);
            let bw = 5 + (s % 6) as i32;
            let bh = if row == 0 {
                10 + (s / 7 % 8) as i32
            } else {
                6 + (s / 7 % 6) as i32
            };
            let bw = bw.min(x1 - x);
            c.rect(x, base - bh, bw, bh, color);
            // Snow on the roof.
            c.rect(x, base - bh, bw, 1, rgb(0xc8d4e8));
            for wy in (base - bh + 2..base - 1).step_by(3) {
                for wx in (x + 1..x + bw - 1).step_by(2) {
                    if hash(s.wrapping_add((wx * 31 + wy) as u64)).is_multiple_of(3) {
                        c.set(wx, wy, rgb(0xffd98a));
                    }
                }
            }
            x += bw;
        }
    }
    // Snow in two layers, as Toyoi does it: small slow flakes far off, bigger
    // faster ones near, each drifting side to side as it falls.
    for (layer, count, cycles, size) in [(0u64, 70u64, 1u64, 1), (1, 28, 2, 2)] {
        for i in 0..count {
            let seed = i * 3 + layer * 1000;
            let fall = phase_at(frame, cycles, unit(seed));
            let y = y0 as f64 + fall * (y1 - y0 + 2) as f64;
            let drift = (phase_at(frame, cycles * 2, unit(seed + 1)) * TAU).sin() * 1.5;
            let x = x0 as f64 + unit(seed + 2) * (x1 - x0) as f64 + drift;
            let (fx, fy) = (x.round() as i32, y.round() as i32);
            for dx in 0..size {
                for dy in 0..size {
                    let (px, py) = (fx + dx, fy + dy);
                    if (x0..x1).contains(&px) && (y0..y1).contains(&py) {
                        c.blend(px, py, SNOW, if size == 1 { 0.6 } else { 0.85 });
                    }
                }
            }
        }
    }
    // Mullions, the sill, and snow banked along the bottom of the glass.
    c.rect((x0 + x1) / 2, y0, 2, y1 - y0, rgb(0x3a2418));
    c.rect(x0, (y0 + y1) / 2 - 2, x1 - x0, 1, rgb(0x3a2418));
    for x in x0..x1 {
        let bank = 1 + (hash(x as u64 / 3 + 90) % 2) as i32;
        c.rect(x, y1 - bank, 1, bank, SNOW);
    }
    c.rect(x0 - 4, y1 + 2, x1 - x0 + 8, 2, rgb(0x5a3a26));
}

/// On the desk: a goldfish bowl, books, the lamp, a mug, a plant, and the
/// notebook the writer is working in.
fn desk_things(c: &mut Canvas, frame: u64) {
    // The lamp's light first, pooling on the desk.
    for y in 26..DESK + 3 {
        let half = 4.0 + (y - 26) as f64 * 0.9;
        for x in (84.0 - half) as i32..=(84.0 + half) as i32 {
            let across = ((x as f64 - 84.0).abs() / half).min(1.0);
            c.blend(x, y, WARM, 0.14 * (1.0 - across * across));
        }
    }
    // The lamp: a bent arm, a green shade.
    c.rect(86, DESK - 2, 6, 2, rgb(0x2a2a2a));
    c.line(
        (88.0, (DESK - 2) as f64),
        (86.0, (DESK - 9) as f64),
        rgb(0x2a2a2a),
    );
    c.line(
        (86.0, (DESK - 9) as f64),
        (83.0, (DESK - 12) as f64),
        rgb(0x2a2a2a),
    );
    c.rect(79, DESK - 14, 7, 3, rgb(0x2f6b5e));
    c.rect(80, DESK - 11, 5, 1, rgb(0xfff1c2));

    // The goldfish bowl: water, a fish going round, a weed.
    let (bx, by) = (8.0, (DESK - 6) as f64);
    c.disc(bx, by, 5.5, rgb(0xcfe6ff));
    c.disc(bx, by + 0.5, 5.0, rgb(0x6fa8d8));
    c.rect(4, DESK - 10, 9, 1, rgb(0xcfe6ff));
    c.rect(6, DESK - 4, 1, 3, rgb(0x4f8a4b));
    c.rect(7, DESK - 3, 1, 2, rgb(0x4f8a4b));
    let swim = phase(frame, 1);
    let fx = bx + (swim * TAU).sin() * 2.5;
    let fy = by + (swim * TAU * 2.0).sin() * 0.8;
    let heading = (swim * TAU).cos() > 0.0;
    let (fx, fy) = (fx.round() as i32, fy.round() as i32);
    c.rect(fx - 1, fy, 3, 1, rgb(0xff8a3a));
    c.set(fx + if heading { 2 } else { -2 }, fy, rgb(0xffb070));
    c.set(fx + if heading { -2 } else { 2 }, fy, rgb(0xe86a2a));
    c.rect(2, DESK - 1, 13, 1, rgb(0xa8c8e8));

    // A stack of books, a mug steaming.
    c.rect(18, DESK - 3, 13, 3, rgb(0x7a3b45));
    c.rect(19, DESK - 6, 11, 3, rgb(0xc79a3b));
    c.rect(18, DESK - 8, 12, 2, rgb(0x3b5a7a));
    c.rect(66, DESK - 5, 5, 5, rgb(0xe8d8c8));
    c.rect(66, DESK - 5, 5, 1, rgb(0x5a3624));
    c.set(71, DESK - 3, rgb(0xe8d8c8));
    for puff in 0..5u64 {
        let rise = phase_at(frame, 2, puff as f64 / 5.0);
        let px = 68.0 + ((rise * 1.5) * TAU).sin() * 1.2;
        c.blend(
            px.round() as i32,
            (DESK as f64 - 6.0 - rise * 9.0).round() as i32,
            rgb(0xeee8de),
            (1.0 - rise) * 0.5,
        );
    }
    // A small cactus.
    c.rect(74, DESK - 3, 4, 3, rgb(0xb5603c));
    c.rect(75, DESK - 7, 2, 4, rgb(0x4f8a4b));
    c.set(74, DESK - 5, rgb(0x4f8a4b));
    // The notebook, open, a little to the writer's right.
    c.rect(46, DESK - 1, 16, 1, rgb(0xf6f0e4));
    c.rect(53, DESK - 1, 1, 1, rgb(0xc8c0b0));
}

/// The writer, from behind: a mustard beanie, a teal hoodie, shoulders over
/// the desk. Their head nods to the beat; their right arm writes, the pencil
/// moving along the notebook.
fn writer(c: &mut Canvas, frame: u64, x: i32, desk: i32) {
    let nod = (phase(frame, 8) < 0.25) as i32;
    let skin = rgb(0xc98a5a);
    // Head: hair under a beanie, an ear either side.
    sprite(
        c,
        x + 4,
        desk - 21 + nod,
        &[
            "...bbbb...",
            "..bBbbBb..",
            ".bbbbbbbb.",
            ".BBBBBBBB.",
            ".hhhhhhhh.",
            "ehhhhhhhhe",
            ".hhhhhhhh.",
            "..hhhhhh..",
            "...ffff...",
        ],
        &[
            ('b', rgb(0xd9a441)),
            ('B', rgb(0xb8862e)),
            ('h', rgb(0x2a1d18)),
            ('e', skin),
            ('f', skin),
        ],
        false,
    );
    c.set(x + 8, desk - 22 + nod, rgb(0xf3e6c8)); // the bobble
    // Hoodie, shoulders hunched over the desk, the hood down at the back.
    sprite(
        c,
        x,
        desk - 13,
        &[
            "...tTTTTTTt...",
            "..tTttttttTt..",
            ".ttttttttttttt",
            "tttttttttttttt",
            "tttttttttttttt",
            "tttttttttttttt",
            "tttttttttttttt",
            "tttttttttttttt",
            ".tttttttttttt.",
            ".tttttttttttt.",
            ".tttttttttttt.",
            ".tttttttttttt.",
            ".tttttttttttt.",
        ],
        &[('t', rgb(0x2f8a82)), ('T', rgb(0x246a64))],
        false,
    );
    // The writing arm, out to the notebook; the pencil travels along a line
    // and back, twice a loop.
    let along = phase(frame, 2);
    let px = 47 + (along * 12.0) as i32;
    c.line(
        ((x + 12) as f64, (desk - 9) as f64),
        (px as f64, (desk - 3) as f64),
        rgb(0x2f8a82),
    );
    c.line(
        ((x + 12) as f64, (desk - 8) as f64),
        (px as f64, (desk - 2) as f64),
        rgb(0x246a64),
    );
    c.rect(px, desk - 3, 2, 2, skin);
    c.line(
        ((px + 1) as f64, (desk - 4) as f64),
        ((px + 3) as f64, (desk - 6) as f64),
        rgb(0xe9c46a),
    );
    // What they have written so far: a line of pencil marks behind the hand.
    for k in 47..px {
        if k % 2 == 0 {
            c.set(k, desk - 1, rgb(0x6a6a7a));
        }
    }
}
