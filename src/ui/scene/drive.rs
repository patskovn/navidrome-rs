//! The synthwave drive: a banded sun, peaks, palms and a car on a rolling
//! grid.

use std::f64::consts::TAU;

use super::*;

/// The drive's anchor. The car's lowest row, bobbing, is the core's row 52;
/// twelve rows of crop are sky and stars, stopping short of the sun.
pub(super) fn drive_origin(c: &Canvas) -> (i32, i32) {
    raised_origin(c, 52, 12)
}

pub(super) fn synthwave_drive(c: &mut Canvas, frame: u64) {
    let (ox, oy) = drive_origin(c);
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
        palm(
            c,
            frame,
            root as f64,
            lean,
            (oy + HEIGHT as i32).min(h),
            oy + 22,
        );
    }

    // The car, seen from behind, bobbing on the road, tail lights pulsing.
    c.origin(ox, oy);
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
