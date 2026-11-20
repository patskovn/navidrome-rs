//! The rainy desk: a cat asleep under a rainy window, and a room built out
//! around it to fill the screen.

use std::f64::consts::TAU;

use super::*;

/// The desk scene's anchor. The cat's lowest row is the core's row 39; four
/// rows of crop cost one row of the window frame and nothing else.
pub(super) fn desk_origin(c: &Canvas) -> (i32, i32) {
    raised_origin(c, 39, 4)
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

pub(super) fn rainy_desk(c: &mut Canvas, frame: u64) {
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
