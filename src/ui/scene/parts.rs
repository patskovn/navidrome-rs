//! Pieces shared between scenes: sprites drawn from text, rain, building
//! fronts along a street, small people, and scrolling layers.

use super::*;

/// Draws a picture written as rows of characters, one per pixel, each looked
/// up in `palette`. Anything not in the palette — by convention `.` — is
/// transparent. `flip` mirrors it left to right, for facing the other way.
pub(super) fn sprite(
    c: &mut Canvas,
    x: i32,
    y: i32,
    rows: &[&str],
    palette: &[(char, Rgb)],
    flip: bool,
) {
    for (dy, row) in rows.iter().enumerate() {
        let chars: Vec<char> = row.chars().collect();
        let width = chars.len() as i32;
        for (dx, ch) in chars.iter().enumerate() {
            if let Some(&(_, color)) = palette.iter().find(|(key, _)| key == ch) {
                let dx = if flip {
                    width - 1 - dx as i32
                } else {
                    dx as i32
                };
                c.set(x + dx, y + dy as i32, color);
            }
        }
    }
}

/// Rain between rows `y0..y1` and columns `x0..x1`: `count` streaks, each
/// falling a whole number of times per loop from its own start, three pixels
/// long and fading upwards.
#[allow(clippy::too_many_arguments)]
pub(super) fn rain(
    c: &mut Canvas,
    frame: u64,
    (x0, y0, x1, y1): (i32, i32, i32, i32),
    count: usize,
    color: Rgb,
    alpha: f64,
    salt: u64,
) {
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for i in 0..count as u64 {
        let seed = i * 7 + salt * 100_003;
        let x = x0 + (hash(seed) % (x1 - x0) as u64) as i32;
        let falls = 2 + hash(seed + 1) % 3;
        let fall = phase_at(frame, falls, unit(seed + 2));
        let y = y0 as f64 + fall * (y1 - y0 + 3) as f64;
        for (k, a) in [(0, 1.0), (1, 0.6), (2, 0.3)] {
            let yy = y as i32 - k;
            if (y0..y1).contains(&yy) {
                c.blend(x, yy, color, alpha * a);
            }
        }
    }
}

/// How a person looks: picked by `seed` from small palettes, so a crowd
/// varies without anyone being drawn twice by hand.
#[derive(Clone, Copy)]
pub(super) struct Look {
    pub skin: Rgb,
    pub hair: Rgb,
    pub coat: Rgb,
    pub trousers: Rgb,
    pub umbrella: Rgb,
}

pub(super) fn look(seed: u64) -> Look {
    const SKIN: [u32; 5] = [0xf1c7a3, 0xd9a07a, 0xa86b4a, 0x6e4430, 0xe8b590];
    const HAIR: [u32; 6] = [0x2a1d18, 0x5a3a22, 0xc9a04a, 0x8a3324, 0x1a1a24, 0xb8b0a8];
    const COAT: [u32; 8] = [
        0x3b5a7a, 0x7a3b45, 0x4f6b4a, 0xc79a3b, 0x8a5a9e, 0x2f4858, 0xb5603c, 0x9aa0aa,
    ];
    const TROUSERS: [u32; 4] = [0x23232e, 0x2f3a5c, 0x4a3a2c, 0x3a3a44];
    const UMBRELLA: [u32; 6] = [0xd1495b, 0x2f6b9a, 0xe9c46a, 0x2a2a2a, 0x3fa39a, 0x8a5a9e];
    let pick = |n: &[u32], k: u64| {
        rgb(n[(hash(seed.wrapping_mul(13).wrapping_add(k)) % n.len() as u64) as usize])
    };
    Look {
        skin: pick(&SKIN, 1),
        hair: pick(&HAIR, 2),
        coat: pick(&COAT, 3),
        trousers: pick(&TROUSERS, 4),
        umbrella: pick(&UMBRELLA, 5),
    }
}

/// A person walking under an umbrella, feet on row `feet`, `x` their left
/// edge; 7 wide and 12 tall. `step` swaps the legs.
pub(super) fn walker(c: &mut Canvas, x: i32, feet: i32, who: Look, facing_right: bool, step: bool) {
    let legs: [&str; 2] = if step {
        ["..t.t..", "..t..t."]
    } else {
        ["..tt...", "...t..."]
    };
    let rows = [
        "..uuu..", ".uuuuu.", "uuuuuuu", "...s...", "..hhs..", "..fhs..", ".cccC..", ".cccc..",
        ".cccc..", ".ccc...", legs[0], legs[1],
    ];
    sprite(
        c,
        x,
        feet - 11,
        &rows,
        &[
            ('u', who.umbrella),
            ('s', rgb(0x2a2a2a)),
            ('h', who.hair),
            ('f', who.skin),
            ('c', who.coat),
            ('C', who.skin),
            ('t', who.trousers),
        ],
        !facing_right,
    );
}

/// Where column `x` falls in a layer `period` pixels long that scrolls left
/// by `cycles` whole periods per loop. The layer is drawn by sampling its
/// content at this position, so it repeats seamlessly at any width, and the
/// loop closes because each scroll is a whole number of periods.
pub(super) fn scrolled(x: i32, period: i32, frame: u64, cycles: u64) -> i32 {
    let offset = (phase(frame, cycles) * period as f64).floor() as i32;
    (x + offset).rem_euclid(period)
}

/// One building front for [`street`].
fn facade(c: &mut Canvas, frame: u64, x: i32, width: i32, ground: i32, top: i32, seed: u64) {
    const BODY: [u32; 6] = [0x5a3532, 0x2f3a5c, 0x6e6152, 0x2f5254, 0x4a3a5c, 0x3a3044];
    let body = rgb(BODY[(seed % BODY.len() as u64) as usize]);
    c.rect(x, top, width, ground - top, body);
    // A cornice along the roof, and a dark seam between neighbours.
    c.rect(x, top, width, 1, mix(body, rgb(0xffffff), 0.18));
    c.rect(x, top + 1, width, 1, mix(body, rgb(0x000000), 0.3));
    c.rect(
        x + width - 1,
        top,
        1,
        ground - top,
        mix(body, rgb(0x000000), 0.4),
    );
    // A shop on the ground floor: awning, a lit window, a door.
    let shop_top = ground - 8;
    let awnings = [
        (0xd1495b, 0xeee8de),
        (0x3fa39a, 0xeee8de),
        (0x2f6b9a, 0xe9c46a),
        (0x6b4632, 0xd8cfc0),
    ];
    let (a1, a2) = awnings[(seed / 7 % 4) as usize];
    for dx in 0..width - 1 {
        let color = if (dx / 2) % 2 == 0 { a1 } else { a2 };
        c.rect(x + dx, shop_top, 1, 2, rgb(color));
    }
    let glow = rgb([0xffcf7a, 0xffb37a, 0xbfe3ff][(seed / 11 % 3) as usize]);
    let shop_w = (width - 9).max(2);
    c.rect(x + 2, shop_top + 3, shop_w, 4, glow);
    c.rect(
        x + 2,
        shop_top + 3,
        shop_w,
        1,
        mix(glow, rgb(0xffffff), 0.4),
    );
    c.rect(
        x + width - 6,
        shop_top + 3,
        3,
        5,
        mix(body, rgb(0x000000), 0.5),
    );
    // A neon sign over the awning on some, breathing once a loop.
    if seed.is_multiple_of(3) && width >= 14 {
        let neon = rgb([0xff4fd8, 0x7ee0d6, 0xffd27a][(seed / 5 % 3) as usize]);
        let on = 0.55 + 0.45 * wave(frame, 1, unit(seed));
        let sx = x + width / 2 - 4;
        c.rect(sx, shop_top - 3, 8, 2, mix(body, neon, on));
        c.glow(sx as f64 + 4.0, shop_top as f64 - 2.0, 6.0, neon, 0.25 * on);
    }
    // Windows above: a grid, some lit, a few switching through the loop.
    let mut wy = top + 3;
    while wy + 4 < shop_top - 3 {
        let mut wx = x + 2;
        while wx + 3 <= x + width - 2 {
            let w_seed = seed
                .wrapping_mul(1000)
                .wrapping_add((wx - x) as u64 * 31 + (wy - top) as u64);
            let step = (phase(frame, 2) * 2.0) as u64;
            let lit = hash(w_seed).is_multiple_of(2)
                ^ hash(w_seed.wrapping_add(step * 7)).is_multiple_of(9);
            let color = if lit {
                rgb([0xffd98a, 0xffc070, 0xcfe6ff][(hash(w_seed.wrapping_add(3)) % 3) as usize])
            } else {
                mix(body, rgb(0x0a0a18), 0.55)
            };
            c.rect(wx, wy, 3, 4, color);
            wx += 6;
        }
        wy += 7;
    }
}

/// Building fronts standing on `ground`, filling absolute columns `x0..x1`.
/// Widths, heights and looks are hashed by each building's place relative
/// to `anchor` — the core's left edge — so a building keeps its place and
/// look however wide the canvas grows. Tops fall between `min_top` and
/// `max_top`.
pub(super) fn street(
    c: &mut Canvas,
    frame: u64,
    anchor: i32,
    (x0, x1): (i32, i32),
    ground: i32,
    (min_top, max_top): (i32, i32),
    salt: u64,
) {
    let width_of = |k: i64| 16 + (hash((k as u64).wrapping_add(salt * 7919)) % 15) as i32;
    let draw = |c: &mut Canvas, k: i64, x: i32, w: i32| {
        let seed = hash((k as u64).wrapping_mul(31).wrapping_add(salt));
        let span = (max_top - min_top).max(1) as u64;
        let top = min_top + (seed / 3 % span) as i32;
        facade(c, frame, x, w, ground, top, seed);
    };
    // Rightwards from the anchor…
    let (mut k, mut x) = (0i64, anchor);
    while x < x1 {
        let w = width_of(k);
        if x + w > x0 {
            draw(c, k, x, w);
        }
        x += w;
        k += 1;
    }
    // …and leftwards from it.
    let (mut k, mut x) = (-1i64, anchor);
    while x > x0 {
        let w = width_of(k);
        x -= w;
        if x < x1 {
            draw(c, k, x, w);
        }
        k -= 1;
    }
}
