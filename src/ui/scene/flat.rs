//! The sunset flat: a dog asleep on the rug, a sofa, the low sun through the
//! window laying a beam of light across the room, dust floating in it — and
//! the flat built out around them to fill the screen.

use std::f64::consts::TAU;

use super::*;

/// The scene's anchor. Its subject is the dog, whose paws are the core's
/// row 47; the floor grows beneath it.
pub(super) fn flat_origin(c: &Canvas) -> (i32, i32) {
    raised_origin(c, 47, 4)
}

/// Core rows.
const FLOOR: i32 = 40;
/// Furniture beyond the core repeats this far apart.
const SLOT: i32 = 34;

const WALL: Rgb = rgb(0xb98f7a);
const WALL_LOW: Rgb = rgb(0x9a6f62);
const SUN: Rgb = rgb(0xffd9a0);
const BOARD: Rgb = rgb(0x8a5a3c);
const BOARD_DARK: Rgb = rgb(0x6b4430);
const LEAF: Rgb = rgb(0x4f8a4b);
const LEAF_LIT: Rgb = rgb(0x6fb35f);

pub(super) fn sunset_flat(c: &mut Canvas, frame: u64) {
    let (ox, oy) = flat_origin(c);
    let (w, h) = (c.width as i32, c.height as i32);
    let floor = oy + FLOOR;
    c.origin(0, 0);

    // Walls, warmer towards the window; a skirting board; the floor.
    c.gradient(0, w, 0, floor, &[rgb(0x8a6a72), WALL, WALL_LOW]);
    for x in 0..w {
        let d = ((x - (ox + 26)).abs() as f64 / (w as f64 * 0.6)).min(1.0);
        for y in 0..floor {
            c.blend(x, y, rgb(0xffb070), 0.18 * (1.0 - d));
        }
    }
    c.rect(0, floor - 2, w, 2, rgb(0xe8d8c8));
    for y in floor..h {
        let plank = (y - floor) / 2;
        let base = if plank % 2 == 0 {
            BOARD
        } else {
            mix(BOARD, BOARD_DARK, 0.4)
        };
        c.rect(0, y, w, 1, base);
        let mut jx = (hash(plank as u64 + 5) % 19) as i32 - (ox % 19);
        while jx < w {
            c.set(jx, y, BOARD_DARK);
            jx += 19 + (hash_at(jx - ox, plank as u64) % 11) as i32;
        }
    }

    // A ceiling, and a fan turning in it, when there is room above.
    if oy >= 10 {
        c.rect(0, 0, w, 3, rgb(0xe8d8c8));
        c.rect(0, 3, w, 1, rgb(0xc8b8a8));
        ceiling_fan(c, frame, ox + 47, 3, (oy / 2).min(10));
    }

    // Beyond the core, along the walls: furniture by slot.
    let mut slot = (0 - ox).div_euclid(SLOT) - 1;
    loop {
        let x = ox + slot * SLOT;
        if x > w {
            break;
        }
        // The core covers slots 0 to 2.
        if !(0..3).contains(&slot) {
            furniture(c, frame, x + 3, floor, oy, hash_at(slot, 808));
        }
        slot += 1;
    }

    c.origin(ox, oy);
    window(c, frame);

    // On a tall wall: a shelf of books and plants over the sofa, a big print
    // over the window.
    if oy >= 18 {
        wall_shelf(c, frame, 54, -8);
    }
    if oy >= 26 {
        frame_photo(c, 14, -24, 24, 16, 2);
        frame_photo(c, 16, -22, 20, 12, 0);
    }

    // Photos over the sofa.
    for (x, y, wdt, hgt, art) in [(56, 8, 9, 11, 0u64), (68, 6, 13, 9, 1), (84, 9, 8, 8, 2)] {
        frame_photo(c, x, y, wdt, hgt, art);
    }

    // The sofa.
    sofa(c, 50, FLOOR);
    // A side table with a plant, by the sofa.
    c.rect(44, FLOOR - 9, 6, 1, rgb(0x6b4632));
    c.rect(46, FLOOR - 8, 2, 8, rgb(0x6b4632));
    c.rect(44, FLOOR - 13, 5, 4, rgb(0xd8cfc0));
    for (dx, dy) in [
        (0, -1),
        (1, -3),
        (2, -2),
        (3, -4),
        (4, -1),
        (2, -5),
        (-1, -2),
    ] {
        c.set(
            45 + dx,
            FLOOR - 13 + dy,
            if dy < -3 { LEAF_LIT } else { LEAF },
        );
    }

    // The rug, the dog asleep on it, a ball.
    c.origin(0, 0);
    rug(c, ox + 22, floor + 3, 56, (h - floor - 4).clamp(4, 9));
    c.origin(ox, oy);
    dog(c, frame, 28, FLOOR + 8);
    c.disc(64.5, (FLOOR + 6) as f64 + 0.5, 1.6, rgb(0xd1495b));
    c.set(64, FLOOR + 5, rgb(0xff8a9a));

    // Last, the sunbeam over everything it falls on, and dust in it.
    c.origin(0, 0);
    sunbeam(c, frame, ox, oy, w, h);
}

/// The window: a sunset, a low sun, a skyline, clouds drifting across, and
/// curtains either side that stir.
fn window(c: &mut Canvas, frame: u64) {
    let (x0, y0, x1, y1) = (8, 4, 44, 30);
    c.rect(x0 - 2, y0 - 2, x1 - x0 + 4, y1 - y0 + 4, rgb(0xeee4d8));
    c.gradient(
        x0,
        x1,
        y0,
        y1,
        &[rgb(0x5a4a8a), rgb(0xc8607a), rgb(0xff8a5a), rgb(0xffc27a)],
    );
    c.glow(26.0, 23.0, 10.0, rgb(0xfff0b0), 0.4);
    c.disc(26.0, 23.0, 3.6, rgb(0xfff4c8));
    // Clouds: two bands, drifting across once a loop, wrapping inside.
    for (band, (cy, speed, color)) in [(19, 1u64, 0xffb0a0), (11, 2, 0xe890a8)].iter().enumerate() {
        let span = x1 - x0;
        let shift = (phase(frame, *speed) * span as f64) as i32;
        for k in 0..3 {
            let cx = x0 + (k * 13 + band as i32 * 7 + shift).rem_euclid(span);
            for dx in 0..7 {
                let x = x0 + (cx - x0 + dx).rem_euclid(span);
                c.blend(x, *cy, rgb(*color), 0.6);
                if (1..5).contains(&dx) {
                    c.blend(x, *cy - 1, rgb(*color), 0.5);
                }
            }
        }
    }
    // Skyline across the bottom.
    for x in x0..x1 {
        let height = 2 + (hash(x as u64 / 3 + 40) % 6) as i32;
        c.rect(x, y1 - height, 1, height, rgb(0x6a4a6a));
        if hash(x as u64 + 41).is_multiple_of(6) {
            c.set(x, y1 - height + 1, rgb(0xffd98a));
        }
    }
    // Mullions and sill.
    c.rect((x0 + x1) / 2, y0, 1, y1 - y0, rgb(0xeee4d8));
    c.rect(x0, (y0 + y1) / 2, x1 - x0, 1, rgb(0xeee4d8));
    c.rect(x0 - 3, y1 + 1, x1 - x0 + 6, 2, rgb(0xf6ece0));
    // Curtains, their hems swinging a pixel.
    let stir = (phase(frame, 1) * TAU).sin();
    for (cx, dir) in [(x0 - 5, -1.0), (x1 + 1, 1.0)] {
        for y in y0 - 3..y1 + 6 {
            let t = (y - (y0 - 3)) as f64 / (y1 + 9 - y0) as f64;
            let sway = (stir * dir * t * 1.2).round() as i32;
            for dx in 0..4 {
                let fold = if dx % 2 == 0 {
                    rgb(0x3f8a7a)
                } else {
                    rgb(0x2f6b5e)
                };
                c.set(cx + dx + sway, y, fold);
            }
        }
    }
    c.rect(x0 - 6, y0 - 4, x1 - x0 + 12, 1, rgb(0x6b4632));
}

/// A floating shelf: books leaning, a trailing plant, a candle.
fn wall_shelf(c: &mut Canvas, frame: u64, x: i32, y: i32) {
    c.rect(x, y, 36, 2, rgb(0x8a5a3c));
    c.rect(x, y, 36, 1, rgb(0xa06c4a));
    let books = [(0x7a3b45, 6), (0x3b5a7a, 7), (0xc79a3b, 5), (0x4f6b4a, 6)];
    for (k, (color, height)) in books.into_iter().enumerate() {
        c.rect(x + 2 + k as i32 * 3, y - height, 2, height, rgb(color));
    }
    c.rect(x + 16, y - 4, 5, 4, rgb(0xe8d8c8));
    let stir = (phase(frame, 1) * TAU).sin();
    for (k, length) in [(0, 6), (2, 9), (4, 4)] {
        for i in 0..length {
            let sway = (stir * i as f64 / length as f64).round() as i32;
            c.set(
                x + 16 + k + sway,
                y + 1 + i,
                if i % 3 == 0 { LEAF_LIT } else { LEAF },
            );
        }
    }
    c.rect(x + 28, y - 4, 3, 4, rgb(0xf3eadc));
    let flick = wave(frame, 6, 0.3);
    c.set(x + 29, y - 5 - (flick > 0.5) as i32, rgb(0xffd27a));
    c.glow(x as f64 + 29.5, (y - 5) as f64, 4.0, rgb(0xffc86e), 0.25);
}

/// A blue sofa with two cushions, its back against the wall.
fn sofa(c: &mut Canvas, x: i32, floor: i32) {
    let body = rgb(0x4f6b8a);
    let shade = rgb(0x3a5270);
    c.rect(x + 2, floor - 16, 38, 8, body);
    c.rect(x + 2, floor - 16, 38, 1, rgb(0x6a86a6));
    c.rect(x, floor - 11, 4, 9, shade);
    c.rect(x + 38, floor - 11, 4, 9, shade);
    c.rect(x, floor - 11, 4, 1, body);
    c.rect(x + 38, floor - 11, 4, 1, body);
    c.rect(x + 4, floor - 8, 34, 5, body);
    c.rect(x + 4, floor - 8, 34, 1, rgb(0x6a86a6));
    c.rect(x + 20, floor - 8, 1, 5, shade);
    c.rect(x + 2, floor - 3, 38, 1, shade);
    c.rect(x + 3, floor - 2, 2, 2, rgb(0x3a2418));
    c.rect(x + 37, floor - 2, 2, 2, rgb(0x3a2418));
    // Cushions.
    c.rect(x + 6, floor - 13, 7, 5, rgb(0xe9c46a));
    c.rect(x + 6, floor - 13, 7, 1, rgb(0xf6dc8a));
    c.rect(x + 29, floor - 13, 7, 5, rgb(0xd1495b));
    c.rect(x + 29, floor - 13, 7, 1, rgb(0xe8707e));
    // A folded throw over the arm.
    c.rect(x + 37, floor - 12, 5, 5, rgb(0xeee4d8));
    c.rect(x + 37, floor - 10, 5, 1, rgb(0xc8607a));
}

fn frame_photo(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, art: u64) {
    c.rect(x, y, w, h, rgb(0x3a2418));
    c.rect(x + 1, y + 1, w - 2, h - 2, rgb(0xf6ece0));
    match art {
        0 => {
            c.gradient(
                x + 2,
                x + w - 2,
                y + 2,
                y + h - 2,
                &[rgb(0x9ac4ff), rgb(0x6fb35f)],
            );
            c.disc((x + w / 2) as f64, (y + 4) as f64, 1.5, rgb(0xfff0b0));
        }
        1 => {
            // The dog, as a puppy.
            c.rect(x + 4, y + 4, 5, 3, rgb(0xd9a441));
            c.rect(x + 3, y + 3, 2, 2, rgb(0xd9a441));
            c.set(x + 3, y + 3, rgb(0x2a1d18));
        }
        _ => {
            c.rect(x + 2, y + 2, w - 4, h - 4, rgb(0xd1495b));
            c.rect(x + 3, y + 3, w - 6, h - 6, rgb(0xe9c46a));
        }
    }
}

/// An oval rug at `x`, rows `y..y + height`, with a border and a pattern.
fn rug(c: &mut Canvas, x: i32, y: i32, width: i32, height: i32) {
    let (cx, cy) = (
        x as f64 + width as f64 / 2.0,
        y as f64 + height as f64 / 2.0,
    );
    let (rx, ry) = (width as f64 / 2.0, height as f64 / 2.0);
    for yy in y..y + height {
        for xx in x..x + width {
            let e = ((xx as f64 + 0.5 - cx) / rx).powi(2) + ((yy as f64 + 0.5 - cy) / ry).powi(2);
            if e <= 1.0 {
                let color = if e > 0.75 {
                    rgb(0x2f6b5e)
                } else if ((e * 6.0) as i32) % 2 == 0 {
                    rgb(0xe8d8c0)
                } else {
                    rgb(0xd8b890)
                };
                c.set(xx, yy, color);
            }
        }
    }
}

/// A golden retriever asleep on its side, head on its paws, facing left:
/// built from ellipses with a dark outline so it reads against the rug. Its
/// flank rises as it breathes, its tail thumps twice in a row each loop, an
/// ear twitches once.
fn dog(c: &mut Canvas, frame: u64, x: i32, paws: i32) {
    let fur = rgb(0xd9a441);
    let shade = rgb(0xb8862e);
    let light = rgb(0xf0c66a);
    let line = rgb(0x6b4a1e);
    let (x, p) = (x as f64, paws as f64);
    let breath = 0.5 * wave(frame, 2, 0.0);

    // The tail first, behind everything: out from the rump, thumping.
    let thump = phase_at(frame, 1, 0.2);
    let up = thump < 0.05 || (0.1..0.15).contains(&thump);
    let tip = if up { p - 11.0 } else { p - 5.0 };
    c.line((x + 30.0, p - 5.0), (x + 37.0, tip), line);
    c.line((x + 30.0, p - 4.0), (x + 37.0, tip + 1.0), line);
    c.line((x + 30.0, p - 5.0), (x + 36.0, tip + 0.5), fur);

    // Outlines, one pixel bigger than each shape.
    c.ellipse(x + 19.0, p - 5.0, 13.0, 5.6 + breath, line);
    c.ellipse(x + 6.0, p - 6.0, 5.6, 4.8, line);
    c.ellipse(x + 1.5, p - 4.5, 3.8, 2.8, line);
    c.ellipse(x + 26.0, p - 3.0, 4.6, 3.4, line);
    c.rect(x as i32, paws - 2, 13, 3, line);

    // Body, hind leg, head, muzzle.
    c.ellipse(x + 19.0, p - 5.0, 12.0, 4.6 + breath, fur);
    c.ellipse(x + 18.0, p - 8.0 - breath, 7.0, 1.3, light);
    c.ellipse(x + 26.0, p - 3.0, 3.6, 2.4, shade);
    c.ellipse(x + 6.0, p - 6.0, 4.6, 3.8, fur);
    c.ellipse(x + 1.5, p - 4.5, 2.8, 1.8, light);
    // Front paws stretched out under the chin.
    c.rect(x as i32 + 1, paws - 1, 11, 1, light);
    c.set(x as i32 + 1, paws - 1, shade);
    // The floppy ear over the head, twitching once a loop.
    let twitch = if phase_at(frame, 1, 0.6) < 0.04 {
        -1.0
    } else {
        0.0
    };
    c.ellipse(x + 8.0, p - 6.0 + twitch, 1.8, 3.2, shade);
    // Nose, and a closed, contented eye.
    c.rect(x as i32 - 2, paws - 5, 2, 1, rgb(0x2a1d18));
    c.rect(x as i32 + 3, paws - 8, 2, 1, line);
}

/// The beam of low sun from the window across the room and floor, and dust
/// turning in it.
fn sunbeam(c: &mut Canvas, frame: u64, ox: i32, oy: i32, w: i32, h: i32) {
    let (wx0, wx1, wy) = (ox + 8, ox + 44, oy + 30);
    // The beam slants down and to the right from the window's bottom edge.
    for y in wy..h {
        let drop = (y - wy) as f64;
        let x0 = wx0 as f64 + drop * 1.1;
        let x1 = wx1 as f64 + drop * 1.6;
        let fade = (1.0 - drop / (h - wy).max(1) as f64 * 0.6).max(0.0);
        for x in x0 as i32..(x1 as i32).min(w) {
            let edge = ((x as f64 - x0).min(x1 - x as f64) / 3.0).min(1.0);
            c.blend(x, y, SUN, 0.16 * fade * edge);
        }
    }
    // Dust motes drifting in it.
    for i in 0..24u64 {
        let t = phase_at(frame, 1, unit(i + 300));
        let drop = (unit(i + 301) * (h - wy) as f64 * 0.7) + (t * TAU).sin() * 2.0;
        let x0 = wx0 as f64 + drop * 1.1;
        let x1 = wx1 as f64 + drop * 1.6;
        let x = x0 + (x1 - x0) * unit(i + 302) + (t * TAU * 2.0).cos() * 1.5;
        let glint = wave(frame, 2 + i % 2, unit(i + 303));
        c.blend(
            x.round() as i32,
            wy + drop.round() as i32,
            rgb(0xfff4dc),
            0.25 + 0.5 * glint,
        );
    }
}

/// A ceiling fan hanging from `top` at `x`, its blades turning.
fn ceiling_fan(c: &mut Canvas, frame: u64, x: i32, top: i32, drop: i32) {
    c.rect(x, top, 1, drop, rgb(0x3a2418));
    let y = top + drop;
    // Seen edge on, the blades sweep out and back: four positions, sixteen
    // turns a loop.
    let turn = (phase(frame, 16) * 4.0) as usize;
    let (left, right) = ([14, 9, 3, 9][turn], [3, 9, 14, 9][turn]);
    c.rect(x - left, y + 1, left, 2, rgb(0x8a5a3c));
    c.rect(x - left, y + 1, left, 1, rgb(0xa06c4a));
    c.rect(x + 1, y + 1, right, 2, rgb(0x8a5a3c));
    c.rect(x + 1, y + 1, right, 1, rgb(0xa06c4a));
    // The motor housing and a light beneath it.
    c.rect(x - 2, y, 5, 3, rgb(0x6b4632));
    c.rect(x - 1, y + 3, 3, 2, rgb(0xf3eadc));
    c.glow(x as f64 + 0.5, (y + 4) as f64, 6.0, rgb(0xfff1c2), 0.2);
}

/// Furniture against the wall beyond the core, picked by `seed`: a
/// bookcase, a tall plant, a lamp, pictures over a cabinet.
fn furniture(c: &mut Canvas, frame: u64, x: i32, floor: i32, oy: i32, seed: u64) {
    match seed % 4 {
        0 => {
            // A bookcase, full.
            let top = floor - 30;
            c.rect(x, top, 26, 30, rgb(0x6b4632));
            for shelf in 0..4 {
                let sy = top + 2 + shelf * 7;
                c.rect(x + 1, sy, 24, 6, rgb(0x3a2418));
                let mut bx = x + 2;
                let mut n = 0;
                while bx < x + 24 {
                    let s = hash(seed.wrapping_add(shelf as u64 * 41 + n));
                    let bw = 1 + (s % 3) as i32;
                    let bh = 4 + (s / 3 % 3) as i32;
                    let color = rgb([0x7a3b45, 0x3b5a7a, 0xc79a3b, 0x4f6b4a, 0xd8cfc0, 0x8a5a9e]
                        [(s / 7 % 6) as usize]);
                    c.rect(bx, sy + 6 - bh, bw.min(x + 24 - bx), bh, color);
                    bx += bw;
                    n += 1;
                }
            }
        }
        1 => {
            // A tall monstera in a pot, its leaves stirring.
            c.rect(x + 8, floor - 7, 10, 7, rgb(0xe8d8c8));
            c.rect(x + 8, floor - 7, 10, 1, rgb(0xf6ece0));
            let stir = (phase_at(frame, 1, unit(seed)) * TAU).sin();
            for (k, (dx, dy)) in [
                (-6, -20),
                (6, -22),
                (-2, -28),
                (4, -14),
                (-5, -12),
                (8, -30),
            ]
            .iter()
            .enumerate()
            {
                let lx = x + 13 + dx + (stir * (k % 2) as f64).round() as i32;
                let ly = floor + dy;
                c.line(
                    ((x + 13) as f64, (floor - 7) as f64),
                    (lx as f64, (ly + 3) as f64),
                    LEAF,
                );
                c.disc(
                    lx as f64,
                    ly as f64 + 1.5,
                    3.2,
                    if k % 2 == 0 { LEAF } else { LEAF_LIT },
                );
                c.set(lx, ly + 1, rgb(0x3a6a3a));
            }
        }
        2 => {
            // A floor lamp, warm, and a basket.
            c.rect(x + 10, floor - 26, 1, 26, rgb(0x2a2a2a));
            c.rect(x + 7, floor - 1, 7, 1, rgb(0x2a2a2a));
            for (i, y) in (floor - 31..floor - 26).enumerate() {
                c.rect(x + 8 - i as i32 / 2, y, 5 + i as i32, 1, rgb(0xf6dca0));
            }
            c.glow(
                x as f64 + 10.5,
                (floor - 24) as f64,
                12.0,
                rgb(0xffd090),
                0.3,
            );
            c.rect(x + 16, floor - 6, 8, 6, rgb(0xb08a5a));
            c.rect(x + 16, floor - 6, 8, 1, rgb(0xc9a46e));
        }
        _ => {
            // A low cabinet, a record player on it, pictures above.
            c.rect(x, floor - 10, 26, 10, rgb(0x8a5a3c));
            c.rect(x, floor - 10, 26, 1, rgb(0xa06c4a));
            c.rect(x + 12, floor - 9, 1, 8, rgb(0x6b4430));
            c.rect(x + 3, floor - 13, 12, 3, rgb(0x2a2a2a));
            c.disc(x as f64 + 8.5, (floor - 13) as f64, 3.0, rgb(0x161616));
            c.set(x + 8, floor - 13, rgb(0xd1495b));
            let top = (floor - 30).max(oy + 2);
            frame_photo(c, x + 2, top, 10, 12, seed / 4 % 3);
            frame_photo(c, x + 15, top + 3, 8, 8, (seed / 4 + 1) % 3);
        }
    }
}
