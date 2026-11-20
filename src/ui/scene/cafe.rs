//! The coffee bar: a barista working the espresso machine behind the counter,
//! a chalkboard menu, a pastry case, and customers at little tables along the
//! room — built out to fill the screen.

use std::f64::consts::TAU;

use super::parts::{Look, look, rain, sprite};
use super::*;

/// The scene's anchor. Its subject is the barista at the counter, whose
/// front's bottom is the core's row 44.
pub(super) fn bar_origin(c: &Canvas) -> (i32, i32) {
    raised_origin(c, 44, 4)
}

/// Core rows.
const COUNTER: i32 = 31; // the counter's top
const FLOOR: i32 = 44;
/// Wall features repeat this far apart beyond the core.
const BAY: i32 = 44;

const TILE: Rgb = rgb(0x2f5a54);
const GROUT: Rgb = rgb(0x22433f);
const WOOD: Rgb = rgb(0x6b4632);
const WOOD_LIT: Rgb = rgb(0x8a5a3c);
const CHROME: Rgb = rgb(0xc8ccd6);
const CHROME_DARK: Rgb = rgb(0x7a7e8c);
const CUP: Rgb = rgb(0xf3eadc);
const COFFEE: Rgb = rgb(0x5a3624);
const STEAM: Rgb = rgb(0xeee8de);

pub(super) fn coffee_bar(c: &mut Canvas, frame: u64) {
    let (ox, oy) = bar_origin(c);
    let (w, h) = (c.width as i32, c.height as i32);
    let floor = oy + FLOOR;
    c.origin(0, 0);

    // Back wall: subway tiles from the ceiling to the floor.
    for y in 0..floor {
        for x in 0..w {
            let row = (y - oy).div_euclid(3);
            let brick = (x - ox + if row % 2 == 0 { 0 } else { 3 }).rem_euclid(6);
            let grout = (y - oy).rem_euclid(3) == 2 || brick == 5;
            c.set(x, y, if grout { GROUT } else { TILE });
        }
    }
    // A warm wash from the lamps over the top of the wall.
    for y in 0..floor {
        let t = 1.0 - (y as f64 / floor.max(1) as f64);
        for x in 0..w {
            c.blend(x, y, rgb(0xffc86e), 0.05 + t * 0.06);
        }
    }

    // Beyond the core, along the wall: rainy windows and shelves by turns,
    // and a table with customers in front of each.
    let first = ox - BAY * ((ox + BAY - 1) / BAY + 1);
    let mut bay = first;
    while bay < w + BAY {
        let index = (bay - ox).div_euclid(BAY);
        if !(0..=1).contains(&index) && bay + BAY > 0 {
            let seed = hash_at(index, 404);
            if index % 2 == 0 {
                back_window(c, frame, bay + 8, oy + 6, seed);
            } else {
                shelves(c, bay + 6, oy + 8, seed);
            }
        }
        bay += BAY;
    }

    // Lamps hanging over the bar and along the room.
    if oy > 0 {
        c.gradient(0, w, 0, oy.min(4), &[rgb(0x1c120e), rgb(0x2e1e16)]);
    }
    let mut x = first + BAY / 2;
    while x < w + BAY {
        lamp(c, frame, x, (oy - 1).max(3) + 2);
        x += BAY;
    }

    // Floor: checker tiles.
    for y in floor..h {
        for x in 0..w {
            let tile = ((x - ox).div_euclid(4) + (y - floor).div_euclid(2)).rem_euclid(2) == 0;
            c.set(x, y, if tile { rgb(0x4a3a30) } else { rgb(0x6a5848) });
        }
    }
    for x in 0..w {
        c.blend(x, floor, rgb(0x000000), 0.4);
        c.blend(x, floor + 1, rgb(0x000000), 0.2);
    }

    // Tables along the room beyond the core, people at them.
    let mut bay = first;
    while bay < w + BAY {
        let index = (bay - ox).div_euclid(BAY);
        if !(0..=1).contains(&index) {
            table(c, frame, bay + BAY / 2, floor + 6, hash_at(index, 405));
        }
        bay += BAY;
    }

    // On a tall floor, a nearer row of tables, in front of the counter too.
    if h - floor >= 24 {
        let mut x = first + BAY;
        while x < w + BAY {
            table(c, frame, x, h - 3, hash_at((x - ox).div_euclid(BAY), 406));
            x += BAY;
        }
    }

    // Plants hanging from the ceiling between the lamps, on a tall wall.
    if oy >= 16 {
        let mut x = first + 6;
        while x < w + BAY {
            super::desk::hanging_plant(c, frame, x, 0, oy - 6);
            x += BAY;
        }
    }

    // --- The core: menu, shelf, the bar itself. ---
    c.origin(ox, oy);
    if oy >= 12 {
        sign(c, 33, -11);
    }
    menu(c, 32, 2);
    shelves(c, 66, 9, 7);
    shelves(c, 2, 9, 3);

    // The barista, behind the counter.
    barista(c, frame, 38, COUNTER);

    // Counter: a marble top, a wooden front with panels.
    c.rect(0, COUNTER, 96, 2, rgb(0xd8d0c8));
    c.rect(0, COUNTER, 96, 1, rgb(0xeee8e0));
    c.rect(0, COUNTER + 2, 96, FLOOR - COUNTER - 2, WOOD);
    c.rect(0, COUNTER + 2, 96, 1, rgb(0x3a2418));
    for x in (4..96).step_by(12) {
        c.rect(x, COUNTER + 4, 9, FLOOR - COUNTER - 6, WOOD_LIT);
        c.rect(x, COUNTER + 4, 9, 1, mix(WOOD_LIT, rgb(0xffffff), 0.15));
    }
    espresso_machine(c, frame, 12, COUNTER);
    grinder(c, 52, COUNTER);
    pastry_case(c, 64, COUNTER);

    // A customer waiting at the counter, seen from behind.
    waiting(c, frame, 70, FLOOR + 6, look(901));
}

/// A painted board over the menu: COFFEE in cream letters.
fn sign(c: &mut Canvas, x: i32, y: i32) {
    const C: [&str; 5] = ["###", "#..", "#..", "#..", "###"];
    const O: [&str; 5] = ["###", "#.#", "#.#", "#.#", "###"];
    const F: [&str; 5] = ["###", "#..", "##.", "#..", "#.."];
    const E: [&str; 5] = ["###", "#..", "##.", "#..", "###"];
    c.rect(x - 2, y, 29, 9, rgb(0x3a2418));
    c.rect(x - 1, y + 1, 27, 7, rgb(0x6b2f2a));
    for (k, letter) in [C, O, F, F, E, E].iter().enumerate() {
        for (dy, row) in letter.iter().enumerate() {
            for (dx, ch) in row.chars().enumerate() {
                if ch == '#' {
                    c.set(
                        x + 1 + k as i32 * 4 + dx as i32,
                        y + 2 + dy as i32,
                        rgb(0xf3e6c8),
                    );
                }
            }
        }
    }
}

/// A chalkboard menu in a wooden frame: lines of chalk, a little cup.
fn menu(c: &mut Canvas, x: i32, y: i32) {
    c.rect(x, y, 30, 15, rgb(0x5a3a26));
    c.rect(x + 1, y + 1, 28, 13, rgb(0x1e2622));
    let chalk = rgb(0xe8e4dc);
    for (k, len) in [(0, 14), (1, 10), (2, 12), (3, 8)] {
        c.rect(x + 3, y + 3 + k * 3, len, 1, chalk);
        c.rect(x + 20, y + 3 + k * 3, 3, 1, rgb(0xe9c46a));
    }
    // A cup doodled in the corner.
    c.rect(x + 24, y + 9, 3, 2, chalk);
    c.set(x + 27, y + 9, chalk);
    c.set(x + 25, y + 7, chalk);
}

/// Wall shelves with cups, jars of beans and a plant, varied by `seed`.
fn shelves(c: &mut Canvas, x: i32, y: i32, seed: u64) {
    for (k, sy) in [y, y + 9].into_iter().enumerate() {
        c.rect(x, sy, 26, 1, WOOD_LIT);
        c.rect(x, sy + 1, 26, 1, rgb(0x3a2418));
        let mut ix = x + 1;
        let mut n = 0u64;
        while ix < x + 24 {
            let s = hash(seed.wrapping_mul(7).wrapping_add(k as u64 * 31 + n));
            match s % 4 {
                0 => {
                    // A stack of cups.
                    c.rect(ix, sy - 3, 4, 3, CUP);
                    c.rect(ix, sy - 4, 4, 1, mix(CUP, rgb(0x000000), 0.15));
                    ix += 5;
                }
                1 => {
                    // A jar of beans.
                    c.rect(ix, sy - 5, 4, 5, rgb(0xcfe6ff));
                    c.rect(ix, sy - 3, 4, 3, rgb(0x6b4024));
                    c.rect(ix, sy - 6, 4, 1, rgb(0x9a6a48));
                    ix += 5;
                }
                2 => {
                    // A little plant trailing over the edge.
                    c.rect(ix, sy - 3, 4, 3, rgb(0xb5603c));
                    for (dx, dy) in [(0, -4), (1, -5), (2, -4), (3, -5), (-1, 1), (4, 2)] {
                        c.set(ix + dx, sy + dy, rgb(0x4f8a4b));
                    }
                    ix += 6;
                }
                _ => ix += 3,
            }
            n += 1;
        }
    }
}

/// A window in the back wall, rain against a night street, its frame wood.
fn back_window(c: &mut Canvas, frame: u64, x: i32, y: i32, seed: u64) {
    let (ww, wh) = (28, 20);
    c.rect(x - 1, y - 1, ww + 2, wh + 2, rgb(0x3a2418));
    c.gradient(x, x + ww, y, y + wh, &[rgb(0x141a36), rgb(0x2a2a52)]);
    // A building opposite, lit windows.
    c.rect(x, y + 6, ww, wh - 6, rgb(0x1c2038));
    for k in 0..8 {
        if hash(seed + k).is_multiple_of(2) {
            c.rect(
                x + 2 + (k as i32 % 4) * 7,
                y + 8 + (k as i32 / 4) * 5,
                3,
                3,
                rgb(0xffd98a),
            );
        }
    }
    rain(
        c,
        frame,
        (x, y, x + ww, y + wh),
        10,
        rgb(0x9cc0f0),
        0.5,
        seed % 97,
    );
    c.rect(x + ww / 2, y, 1, wh, rgb(0x3a2418));
    c.rect(x - 2, y + wh + 1, ww + 4, 1, WOOD_LIT);
}

/// A brass lamp on a cord from the ceiling at column `x`, down to `bottom`.
fn lamp(c: &mut Canvas, frame: u64, x: i32, bottom: i32) {
    let sway = (phase_at(frame, 1, unit(x as u64 % 101)) * TAU).sin() * 0.5;
    c.line(
        (x as f64, 0.0),
        (x as f64 + sway, (bottom - 3) as f64),
        rgb(0x0e0a08),
    );
    let sx = (x as f64 + sway).round() as i32;
    c.glow(
        sx as f64 + 0.5,
        bottom as f64 + 2.0,
        14.0,
        rgb(0xffc86e),
        0.28,
    );
    c.rect(sx - 2, bottom - 3, 5, 1, rgb(0xb8862e));
    c.rect(sx - 3, bottom - 2, 7, 2, rgb(0xd9a441));
    c.rect(sx - 1, bottom, 3, 1, rgb(0xfff1c2));
}

/// The espresso machine on the counter: chrome, a gauge, cups warming on
/// top, a shot pouring into a cup, and the steam wand puffing now and then.
fn espresso_machine(c: &mut Canvas, frame: u64, x: i32, counter: i32) {
    let top = counter - 13;
    c.rect(x, top, 22, 13, CHROME);
    c.rect(x, top, 22, 1, rgb(0xeef0f6));
    c.rect(x + 1, top + 2, 20, 2, CHROME_DARK);
    // Cups warming on top.
    for k in 0..3 {
        c.rect(x + 3 + k * 6, top - 3, 4, 3, CUP);
    }
    // A pressure gauge.
    c.disc(x as f64 + 4.5, (top + 7) as f64, 2.2, rgb(0xf3eadc));
    c.set(x + 4, top + 6, rgb(0xd1495b));
    // The group head and portafilter, and a cup beneath catching the shot.
    c.rect(x + 11, top + 4, 6, 2, rgb(0x2a2a2a));
    c.rect(x + 17, top + 5, 4, 1, rgb(0x2a2a2a));
    let pour = phase(frame, 1);
    let cup_y = counter - 3;
    c.rect(x + 12, cup_y, 4, 3, CUP);
    if pour < 0.6 {
        // The stream, thin and dark, flickering down.
        for y in top + 6..cup_y {
            if (y + (frame % 2) as i32) % 3 != 0 {
                c.set(x + 13 + (y % 2), y, COFFEE);
            }
        }
    }
    let fill = ((pour.min(0.6) / 0.6) * 2.0).round() as i32;
    if fill > 0 {
        c.rect(x + 12, cup_y + 3 - fill, 4, fill, COFFEE);
    }
    // Steam off the fresh shot, and faintly off the warming cups.
    for (k, (sx, sy, strength)) in [
        (x + 13, cup_y - 1, 0.5),
        (x + 4, top - 4, 0.25),
        (x + 10, top - 4, 0.25),
        (x + 16, top - 4, 0.25),
    ]
    .into_iter()
    .enumerate()
    {
        for puff in 0..4u64 {
            let rise = phase_at(frame, 2, k as f64 * 0.27 + puff as f64 / 4.0);
            let px = sx as f64 + ((rise * 1.5 + k as f64) * TAU).sin();
            let py = sy as f64 - rise * 7.0;
            c.blend(
                px.round() as i32,
                py.round() as i32,
                STEAM,
                (1.0 - rise) * strength,
            );
        }
    }
    // The steam wand, and a puff from it twice a loop.
    c.rect(x + 20, top + 6, 1, 5, CHROME_DARK);
    let puff = phase(frame, 2);
    if puff < 0.4 {
        for k in 0..6 {
            let t = (puff / 0.4 + k as f64 / 6.0).fract();
            let px = x as f64 + 21.0 + (t * TAU + k as f64).sin() * 1.5 + t * 2.0;
            let py = (top + 10) as f64 - t * 9.0;
            c.blend(px.round() as i32, py.round() as i32, STEAM, (1.0 - t) * 0.7);
        }
    }
}

/// A coffee grinder: a hopper of beans on a squat body.
fn grinder(c: &mut Canvas, x: i32, counter: i32) {
    c.rect(x + 1, counter - 13, 6, 5, rgb(0xcfe6ff));
    c.rect(x + 1, counter - 10, 6, 2, rgb(0x6b4024));
    c.rect(x, counter - 8, 8, 8, rgb(0x2a2a30));
    c.rect(x, counter - 8, 8, 1, rgb(0x4a4a54));
    c.rect(x + 2, counter - 4, 4, 2, rgb(0x8a8a9a));
}

/// A lit glass case of pastries.
fn pastry_case(c: &mut Canvas, x: i32, counter: i32) {
    let top = counter - 9;
    c.rect(x, top, 28, 9, rgb(0x3a2418));
    c.rect(x + 1, top + 1, 26, 7, rgb(0xfff4dc));
    c.rect(x + 1, top + 4, 26, 1, rgb(0xd8cfc0));
    let treats = [
        (0xd08a3e, 0xa8642a),
        (0x8a4a2a, 0x5a2a1a),
        (0xf6c6d4, 0xd1495b),
        (0xe9c46a, 0xc89a3a),
    ];
    for (k, (fill, edge)) in treats.into_iter().enumerate() {
        let tx = x + 2 + k as i32 * 6;
        c.rect(tx, top + 2, 4, 2, rgb(fill));
        c.set(tx + 1, top + 2, rgb(edge));
        c.rect(tx + 1, top + 5, 4, 2, rgb(treats[(k + 1) % 4].0));
    }
    for dy in 1..8 {
        c.blend(x + 3, top + dy, rgb(0xffffff), 0.35);
    }
}

/// The barista behind the counter, from the waist up: shirt, apron, one arm
/// on the machine's lever, pulling it down twice a loop; a blink now and then.
fn barista(c: &mut Canvas, frame: u64, x: i32, counter: i32) {
    let skin = rgb(0xd9a07a);
    let rows = [
        "...hhhhh...",
        "..hhhhhhh..",
        "..hfffffh..",
        "..ffffffff.",
        "...fffff...",
        "....fff....",
        "..ssaaass..",
        ".sssaaasss.",
        ".sssaaasss.",
        ".sssaaasss.",
        ".sssaaasss.",
        "..ssaaass..",
        "..ssaaass..",
        "..ssaaass..",
    ];
    let top = counter - rows.len() as i32;
    sprite(
        c,
        x,
        top,
        &rows,
        &[
            ('h', rgb(0x2a1d18)),
            ('f', skin),
            ('s', rgb(0xeee8de)),
            ('a', rgb(0x3a3a44)),
        ],
        false,
    );
    // Eyes, closing for a blink once a loop; a little smile.
    let blink = phase_at(frame, 1, 0.3) < 0.03;
    if !blink {
        c.set(x + 4, top + 3, rgb(0x2a1d18));
        c.set(x + 7, top + 3, rgb(0x2a1d18));
    }
    c.rect(x + 5, top + 4, 2, 1, mix(skin, rgb(0x8a3324), 0.5));
    // The apron strap round the neck.
    c.set(x + 4, top + 6, rgb(0x3a3a44));
    c.set(x + 6, top + 6, rgb(0x3a3a44));
    // The arm out to the lever, up then pulled down.
    let down = phase(frame, 2) < 0.5;
    let hand = if down {
        (x - 6, top + 9)
    } else {
        (x - 6, top + 6)
    };
    c.line(
        ((x + 1) as f64, (top + 7) as f64),
        (hand.0 as f64, hand.1 as f64),
        rgb(0xeee8de),
    );
    c.line(
        ((x + 1) as f64, (top + 8) as f64),
        (hand.0 as f64, (hand.1 + 1) as f64),
        rgb(0xeee8de),
    );
    c.rect(hand.0 - 1, hand.1, 2, 2, skin);
    // The other hand resting on the counter.
    c.rect(x + 10, top + 10, 2, 2, skin);
}

/// A customer standing at the counter, seen from behind, shifting their
/// weight once a loop.
fn waiting(c: &mut Canvas, frame: u64, x: i32, feet: i32, who: Look) {
    let shift = (phase(frame, 1) < 0.5) as i32;
    let rows = [
        "..hhhh..", ".hhhhhh.", ".hhhhhh.", "..hhhh..", ".cccccc.", "cccccccc", "cccccccc",
        "cccccccc", "cccccccc", "cccccccc", ".cccccc.", ".tt..tt.", ".tt..tt.", ".tt..tt.",
        ".tt..tt.", ".ss..ss.",
    ];
    sprite(
        c,
        x + shift,
        feet - rows.len() as i32 + 1,
        &rows,
        &[
            ('h', who.hair),
            ('c', who.coat),
            ('t', who.trousers),
            ('s', rgb(0x1a1a1a)),
        ],
        false,
    );
}

/// A round café table with one or two customers, by `seed`: sipping, typing
/// on a laptop, reading.
fn table(c: &mut Canvas, frame: u64, x: i32, floor: i32, seed: u64) {
    let top = floor - 7;
    // People first, so the tabletop sits in front of their laps.
    let left = look(seed);
    seated(c, frame, x - 13, floor, left, true, seed);
    if seed.is_multiple_of(2) {
        let right = look(seed.wrapping_add(1));
        seated(c, frame, x + 6, floor, right, false, seed.wrapping_add(3));
    }
    c.rect(x - 7, top, 15, 1, rgb(0xd8d0c8));
    c.rect(x - 6, top + 1, 13, 1, rgb(0x9a9088));
    c.rect(x, top + 2, 1, floor - top - 3, rgb(0x2a2a30));
    c.rect(x - 3, floor - 1, 7, 1, rgb(0x2a2a30));
    // On the table: a cup, and a laptop for some.
    c.rect(x - 4, top - 2, 3, 2, CUP);
    if (seed / 5).is_multiple_of(3) {
        let glow = 0.6 + 0.4 * wave(frame, 3, unit(seed));
        c.rect(
            x + 1,
            top - 5,
            5,
            4,
            mix(rgb(0x2a3a5a), rgb(0x9ac4ff), glow),
        );
        c.rect(x, top - 1, 7, 1, rgb(0x9aa0aa));
    }
}

/// Someone seated at a table, side on, facing it: `facing_right` toward a
/// table on their right. Every so often they lift their cup.
fn seated(
    c: &mut Canvas,
    frame: u64,
    x: i32,
    floor: i32,
    who: Look,
    facing_right: bool,
    seed: u64,
) {
    let rows = [
        "..hhh..", ".hhhhf.", ".hhfff.", "...ff..", "..cccc.", ".ccccc.", ".ccccc.", ".cccc..",
        ".ttttt.", "....t..", "....t..", "...ss..",
    ];
    let y = floor - rows.len() as i32;
    sprite(
        c,
        x,
        y,
        &rows,
        &[
            ('h', who.hair),
            ('f', who.skin),
            ('c', who.coat),
            ('t', who.trousers),
            ('s', rgb(0x1a1a1a)),
        ],
        !facing_right,
    );
    // A stool under them.
    c.rect(x + 1, y + 9, 5, 1, rgb(0x6b4632));
    c.rect(x + 3, y + 10, 1, 2, rgb(0x2a2a30));
    // Sipping: the arm lifts the cup to the mouth for a moment.
    let sip = phase_at(frame, 1, unit(seed)) < 0.15;
    let hand_x = if facing_right { x + 6 } else { x };
    if sip {
        c.rect(hand_x, y + 2, 2, 2, CUP);
    } else {
        c.set(hand_x, y + 6, who.skin);
    }
}
