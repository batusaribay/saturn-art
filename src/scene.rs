//! Saturn, rendered as colored ASCII cells.
//!
//! The scene is a unit sphere (the planet) with a thin annulus in its
//! equatorial plane (the rings), lit by a directional sun.  An orthographic
//! camera sits above the ring plane, so the rings show their classic ellipse
//! and cast a shadow band across the planet.
//!
//! Per cell we shoot one ray, intersect the sphere and the ring plane, keep
//! whichever hit is closer, and shade it:
//!
//!   * planet: latitude/longitude texture -> solar illumination -> ring shadow
//!   * rings:  density profile (C/B/A rings + Cassini + Encke) -> opacity blend
//!
//! Colors are real Saturn tones (pale butterscotch belts, cream zones,
//! bluish-gray north polar hood, tan/cream rings) kept in linear light.

use std::f32::consts::PI;
use std::sync::OnceLock;

use crate::color::{Rgb, from_hex};

// ---------------------------------------------------------------------------
// vectors
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl V3 {
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    #[inline]
    pub fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    #[inline]
    pub fn mul(self, k: f32) -> Self {
        Self::new(self.x * k, self.y * k, self.z * k)
    }

    #[inline]
    pub fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    #[inline]
    pub fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    #[inline]
    pub fn norm(self) -> Self {
        let l = self.dot(self).sqrt().max(1e-6);
        self.mul(1.0 / l)
    }

    #[inline]
    pub fn rotate_y(self, a: f32) -> Self {
        let (s, c) = a.sin_cos();
        Self::new(self.x * c - self.z * s, self.y, self.x * s + self.z * c)
    }
}

// ---------------------------------------------------------------------------
// tuning
// ---------------------------------------------------------------------------

/// Planet radius is 1; Saturn's rings are ~2.27 planet radii across.
pub const RING_OUTER: f32 = 2.272;

/// How much of the frame the ring system may fill.
const FRAME_FILL: f32 = 0.70;

const UP: V3 = V3::new(0.0, 1.0, 0.0);

/// Character cells are about half as wide as they are tall on screen.
const CELL_ASPECT: f32 = 0.5;

/// A named glyph ramp, ordered dark to bright.  All are plain ASCII.
pub struct Charset {
    pub name: &'static str,
    pub ramp: &'static [u8],
}

pub const CHARSETS: &[Charset] = &[
    Charset {
        name: "classic",
        ramp: b" .:-=+*#%@",
    },
    Charset {
        name: "detailed",
        ramp: b" .'`^\",:;Il!i~+_-?][}{1)(|\\/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$",
    },
    Charset {
        name: "soft",
        ramp: b" .'`,:;-~+=*#%@",
    },
    Charset {
        name: "blocks",
        ramp: b" .:;+=xX$#%@",
    },
    Charset {
        name: "dense",
        ramp: b" .,:;irsXA253hMHGS#9B&@",
    },
    Charset {
        name: "dots",
        ramp: b" .:;coCO8@",
    },
    Charset {
        name: "minimal",
        ramp: b" .:-=+*#@",
    },
    Charset {
        name: "mixed",
        ramp: b" .-~+=*ox#%@",
    },
];

// ---------------------------------------------------------------------------
// output
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
pub struct Cell {
    pub ch: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// Index in the glyph ramp, used to damp frame-to-frame shimmer.
    pub gi: u8,
}

pub struct Params {
    /// Elevation of the camera above the ring plane, in radians.
    pub tilt: f32,
    /// Azimuth of the camera around the planet, in radians.
    pub azimuth: f32,
    /// Roll of the camera around its own axis, in radians.
    pub roll: f32,
    /// Camera distance in planet radii (drives the perspective).
    pub distance: f32,
    /// Planet rotation angle, in radians.
    pub spin: f32,
    /// Unit vector pointing from the planet toward the sun.
    pub sun: V3,
    /// Time in seconds, used only for the star twinkle.
    pub time: f32,
    pub stars: bool,
    /// Glyph ramp, dark to bright.
    pub ramp: &'static [u8],
}

// ---------------------------------------------------------------------------
// main renderer
// ---------------------------------------------------------------------------

pub fn render(cols: usize, rows: usize, p: &Params, prev: Option<&[Cell]>) -> Vec<Cell> {
    let cols = cols.max(4);
    let rows = rows.max(3);

    // Perspective camera on an angled orbit around the planet.
    let el = p.tilt.clamp(-1.50, 1.50);
    let dist = p.distance.max(RING_OUTER + 1.2);
    let (se, ce) = el.sin_cos();
    let (sa, ca) = p.azimuth.sin_cos();
    let cam = V3::new(dist * sa * ce, dist * se, -dist * ca * ce);
    let fwd = cam.mul(-1.0).norm();
    let right0 = UP.cross(fwd).norm();
    let up0 = fwd.cross(right0).norm();
    // Roll the camera so the rings cut diagonally across the frame.
    let (sr, cr) = p.roll.sin_cos();
    let right = right0.mul(cr).add(up0.mul(sr));
    let up = up0.mul(cr).sub(right0.mul(sr));
    let sun = p.sun.norm();

    // Auto-frame: project the outer ring circle and the planet's silhouette,
    // then pick the field of view that fits them with a margin.
    let mut max_u = 1.0 / (dist * dist - 1.0).max(1e-3).sqrt();
    let mut max_v = max_u;
    for k in 0..96 {
        let a = k as f32 / 96.0 * std::f32::consts::TAU;
        let pt = V3::new(
            RING_OUTER * 1.02 * a.cos(),
            0.0,
            RING_OUTER * 1.02 * a.sin(),
        );
        let d = pt.sub(cam);
        let z = d.dot(fwd);
        if z > 0.1 {
            max_u = max_u.max((d.dot(right) / z).abs());
            max_v = max_v.max((d.dot(up) / z).abs());
        }
    }
    let tan_half = max_v.max(max_u * 2.0 * rows as f32 / cols as f32) / FRAME_FILL;
    let half_w = tan_half * cols as f32 / rows as f32 * CELL_ASPECT;

    let mut cells = vec![Cell::default(); cols * rows];

    // Supersampling: rings are only a cell or two thick at this angle, so
    // single-sample rays would make them flicker and drop out.  Big terminals
    // drop to 2x2 to stay fast.
    const SUB4: [(f32, f32); 4] = [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];
    const SUB9: [(f32, f32); 9] = [
        (-0.333, -0.333),
        (0.0, -0.333),
        (0.333, -0.333),
        (-0.333, 0.0),
        (0.0, 0.0),
        (0.333, 0.0),
        (-0.333, 0.333),
        (0.0, 0.333),
        (0.333, 0.333),
    ];
    let sub: &[(f32, f32)] = if cols * rows <= 32_000 { &SUB9 } else { &SUB4 };
    let inv = 1.0 / sub.len() as f32;

    for j in 0..rows {
        for i in 0..cols {
            let bg = if p.stars {
                star(i, j, p.time)
            } else {
                Rgb::ZERO
            };

            let mut acc = Rgb::ZERO;
            let mut covered = 0.0_f32;
            let mut planet = 0.0_f32;
            for &(ox, oy) in sub {
                let u = ((i as f32 + 0.5 + ox) / cols as f32 * 2.0 - 1.0) * half_w;
                let v = (1.0 - (j as f32 + 0.5 + oy) / rows as f32 * 2.0) * tan_half;

                let dir = fwd.add(right.mul(u)).add(up.mul(v)).norm();

                let (c, cov, on_planet) = shade_ray(cam, dir, p.spin, sun, p.time, bg);
                acc = acc.add(c);
                covered += cov;
                planet += if on_planet { 1.0 } else { 0.0 };
            }

            let mut col = acc.mul(inv);
            covered *= inv;
            planet *= inv;

            // Thin sub-cell features (the rings at the ansae) would otherwise
            // average away into the background.  Recover some of their
            // brightness so they still render as glyphs.
            if covered > 0.03 && covered < 1.0 {
                let boost = (1.0 / covered).powf(0.50).min(2.6);
                col = col.mul(boost);
            }

            // Keep the globe solid: a cell that is mostly planet must never
            // become a blank hole, even on the shadowed limb.
            if planet > 0.55 && col.luma() < 0.012 {
                col = Rgb::new(0.05, 0.055, 0.075);
            }
            // A cell that caught even a sliver of the ring should not fall back
            // to pure background: that is what reads as a black dot in the ring.
            if covered > 0.01 && col.luma() < 0.03 {
                col = col.add(Rgb::new(0.014, 0.013, 0.012));
            }
            let (mut r, mut g, mut b) = col.to_srgb8();
            // Temporal smoothing: feathers sub-cell flicker (chiefly the thin
            // ring sheet sliding across a cell) without blurring the motion.
            if let Some(pc) = prev.and_then(|p| p.get(j * cols + i)) {
                r = ((r as u16 * 6 + pc.r as u16 * 4) / 10) as u8;
                g = ((g as u16 * 6 + pc.g as u16 * 4) / 10) as u8;
                b = ((b as u16 * 6 + pc.b as u16 * 4) / 10) as u8;
            }
            // Only an essentially uncovered cell may draw a star, otherwise
            // stars would show up inside the planet or the ring sheet.
            let is_star = bg.luma() > 0.0 && covered < 0.03;
            let mut gi = 0u8;
            let ch = if is_star {
                // Stars are dots; only a rare few get the brighter glyph.
                if (h2_of(i, j) >> 24) % 24 == 0 {
                    b'+'
                } else {
                    b'.'
                }
            } else {
                let (glyph_ch, index) = glyph_with_index(r, g, b, p.ramp, prev, j * cols + i);
                gi = index;
                if glyph_ch == b' ' && (planet > 0.55 || covered > 0.01) {
                    b'.'
                } else {
                    glyph_ch
                }
            };

            cells[j * cols + i] = Cell { ch, r, g, b, gi };
        }
    }

    cells
}

/// Trace one ray, compositing what it hits over the background `bg`.
/// Also returns how much of the ray was covered by Saturn (0..1).
#[inline]
fn shade_ray(o: V3, dir: V3, spin: f32, sun: V3, time: f32, bg: Rgb) -> (Rgb, f32, bool) {
    let sphere = sphere_hit(o, dir);
    let ring = ring_hit(o, dir, sun, time);

    match sphere {
        Some((t_s, pos)) => {
            let surface = shade_planet(pos, dir, spin, sun);
            // The planet is opaque.  A ring hit in front of it is blended on
            // top; a hit behind it is hidden.
            let col = match ring {
                Some((t_r, alpha, rc)) if t_r < t_s => surface.over(rc, alpha),
                _ => surface,
            };
            (col, 1.0, true)
        }
        None => match ring {
            Some((_, alpha, rc)) => (bg.over(rc, alpha), alpha, false),
            None => (bg, 0.0, false),
        },
    }
}

#[inline]
fn h2_of(col: usize, row: usize) -> u32 {
    hash2(col as u32 ^ 0x9e37, row as u32 ^ 0x85eb)
}

/// Pick an ASCII glyph for an sRGB color.
#[inline]
/// Pick a glyph (and its ramp index) for an sRGB colour.  When the previous
/// frame is available, keep the old index as long as the new one is within a
/// fraction of a step, which stops cells that sit on a threshold from
/// flickering every frame.
fn glyph_with_index(
    r: u8,
    g: u8,
    b: u8,
    ramp: &[u8],
    prev: Option<&[Cell]>,
    cell: usize,
) -> (u8, u8) {
    let lum = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0;
    // The scene's useful range is roughly 0.04..0.86; spread it over the ramp
    // so the lit belts reach the dense glyphs without blowing out to white.
    let v = ((lum - 0.045) / 0.86).clamp(0.0, 1.0).powf(0.85) * (ramp.len() - 1) as f32;
    let mut index = v.round() as usize;
    if let Some(prev) = prev {
        if let Some(pc) = prev.get(cell) {
            let pi = pc.gi as f32;
            // Hysteresis: only move a glyph when the brightness really moved.
            if (v - pi).abs() < 1.05 {
                index = pi as usize;
            }
        }
    }
    let index = index.min(ramp.len() - 1);
    (ramp[index], index as u8)
}

// ---------------------------------------------------------------------------
// geometry
// ---------------------------------------------------------------------------

/// Ray/sphere intersection for the unit sphere at the origin.
/// Returns the nearest positive hit distance and the surface point.
#[inline]
fn sphere_hit(o: V3, d: V3) -> Option<(f32, V3)> {
    let b = o.dot(d);
    let c = o.dot(o) - 1.0;
    let disc = b * b - c;
    if disc <= 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    if t <= 0.0 {
        return None;
    }
    Some((t, o.add(d.mul(t))))
}

/// Ray/ring-plane intersection.  Returns (distance, opacity, color).
#[inline]
fn ring_hit(o: V3, d: V3, sun: V3, time: f32) -> Option<(f32, f32, Rgb)> {
    if d.y.abs() < 1e-6 {
        return None;
    }
    let t = -o.y / d.y;
    if t <= 0.0 {
        return None;
    }
    let q = o.add(d.mul(t));
    let radius = (q.x * q.x + q.z * q.z).sqrt();
    let (alpha, tone) = ring_sample(radius);
    if alpha <= 0.004 {
        return None;
    }

    // Which face are we looking at, and is the sun on that side?
    let n = if d.y < 0.0 {
        V3::new(0.0, 1.0, 0.0)
    } else {
        V3::new(0.0, -1.0, 0.0)
    };
    let ndl = n.dot(sun).max(0.0);
    // Rings are thin and scatter light through, so there is a healthy floor.
    let light = 0.96 + 0.40 * ndl;

    // The planet's shadow falls across the far side of the rings.  It is deep
    // but never pure black: scattered light still reaches the ring particles.
    let shadow = 1.0 - 0.62 * sphere_occlusion(q, sun);

    // Ring particles orbit at Keplerian rates: the inner rings sweep faster
    // than the outer ones, so the brightness pattern shears around the disc.
    let ang = q.z.atan2(q.x);
    let omega = 0.18 * (1.55 / radius).powf(1.5);
    let phase = ang + omega * time;
    // A large-scale brightness asymmetry (plus a finer ripple) makes the
    // ring's rotation readable even when the disc is nearly edge-on.
    let wave =
        (0.86 + 0.26 * (phase + 0.3).sin() + 0.10 * (phase * 3.0 - 1.1).sin()).clamp(0.35, 1.25);

    // Two dark spokes sweep around the ring plane, the way Voyager's did.
    let spoke = 1.0
        - 0.22 * (-(wrap_pi(phase) / 0.30).powi(2)).exp()
        - 0.14 * (-(wrap_pi(phase + 2.4) / 0.20).powi(2)).exp();

    Some((t, alpha, tone.mul(light * shadow * wave * spoke)))
}

/// Soft shadow of the planet: 1 = fully inside the umbra, 0 = lit.
#[inline]
fn sphere_occlusion(p: V3, sun: V3) -> f32 {
    // Ray from p toward the sun: p + s*sun, s > 0.
    let b = p.dot(sun);
    if b >= 0.0 {
        return 0.0; // sun is behind the planet from this point
    }
    let disc = b * b - (p.dot(p) - 1.0);
    if disc <= 0.0 {
        return 0.0;
    }
    // Perpendicular distance from the sun-ray to the planet's center.
    let d = (p.dot(p) - b * b).max(0.0).sqrt();
    // A slightly soft edge reads better than a hard mathematical umbra.
    smoothstep(1.0, 0.93, d)
}

/// Opacity of the ring sheet at a given radius (in planet radii).
#[inline]
fn ring_shadow_on_planet(p: V3, sun: V3) -> f32 {
    if sun.y.abs() < 1e-4 {
        return 0.0;
    }
    let s = -p.y / sun.y;
    if s <= 0.0 {
        return 0.0;
    }
    let q = p.add(sun.mul(s));
    let r = (q.x * q.x + q.z * q.z).sqrt();
    let (a, _) = ring_sample(r);
    a
}

// ---------------------------------------------------------------------------
// planet shading
// ---------------------------------------------------------------------------

fn shade_planet(p: V3, d: V3, spin: f32, sun: V3) -> Rgb {
    let n = p; // unit sphere, so position == normal

    // Texture coordinates rotate with the planet.
    let ps = p.rotate_y(-spin);
    let lat = ps.y.clamp(-1.0, 1.0).asin();
    let lon = ps.z.atan2(ps.x);

    let albedo = surface(lat, lon);

    // Solar illumination, with a softer-than-real terminator for readability.
    let ndl = n.dot(sun).clamp(-1.0, 1.0);
    let diff = smoothstep(-0.02, 0.11, ndl);
    let lambert = 0.34 + 0.66 * ndl.max(0.0);
    let mut light = (0.020 + 0.980 * diff) * lambert * 0.94;

    // Rings throw a shadow band onto the planet.  Near an equinox the band is
    // thinner than a character cell, so fade it out rather than let it dither
    // into stray dots.
    let shadow = ring_shadow_on_planet(p, sun);
    light *= 1.0 - 0.72 * shadow * smoothstep(0.03, 0.16, sun.y.abs());

    // Limb darkening + a pale atmospheric rim on the sunlit edge.
    // Clamp: at the exact centre of the disc the dot product can round to
    // slightly over 1, and a negative base with a fractional pow is NaN.
    let ndv = n.dot(d.mul(-1.0)).clamp(0.0, 1.0);
    let limb = 0.56 + 0.44 * ndv.powf(0.50);
    let rim = (1.0 - ndv).powf(2.8) * 0.09 * smoothstep(0.0, 0.12, ndl);

    // A whisper of desaturation keeps the butterscotch from going orange.
    let grey = Rgb::new(albedo.luma(), albedo.luma(), albedo.luma());
    let albedo = albedo.lerp(grey, 0.06);

    // Dusk light: a little ringshine keeps the night side readable, which is
    // what preserves the sphere's silhouette in ASCII.
    const DUSK: Rgb = Rgb::new(0.055, 0.060, 0.080);

    // Ringshine: the ring sheet throws a soft warm glow onto the equator.
    let ring_glow = (-(p.y / 0.16).powi(2)).exp() * 0.10;

    albedo
        .mul(light * limb + ring_glow)
        .add(albedo.mul_rgb(DUSK))
        .add(Rgb::new(0.86, 0.79, 0.63).mul(rim))
}

// ---------------------------------------------------------------------------
// Saturn's atmosphere: latitude bands with real-ish colors
// ---------------------------------------------------------------------------

/// Latitude (degrees) -> albedo, from the south pole (-90) to the north (90).
/// Pale butterscotch belts, cream equatorial zone, bluish-gray polar hood.
const BANDS_HEX: &[(f32, u32)] = &[
    (-90.0, 0x7d6454),
    (-82.0, 0x94745f),
    (-74.0, 0xad8870),
    (-66.0, 0xc49c80),
    (-58.0, 0xd3ac8d),
    (-50.0, 0xc29573),
    (-42.0, 0xe6c29c),
    (-34.0, 0xf0d2ac),
    (-27.0, 0xd0a37e),
    (-20.0, 0xe4bf98),
    (-13.0, 0xf4dbb6),
    (-6.0, 0xf7e4bf),
    (1.0, 0xf9e9c6),
    (8.0, 0xf5e0b9),
    (14.0, 0xd2a67f),
    (20.0, 0xe6c39b),
    (27.0, 0xf3ddb8),
    (34.0, 0xd4ab85),
    (41.0, 0xc49a76),
    (48.0, 0xe2c19d),
    (55.0, 0xd8b592),
    (62.0, 0xcfad8c),
    (70.0, 0xc3a186),
    (77.0, 0xb79884),
    (83.0, 0xaa8f80),
    (88.0, 0x9f897d),
    (90.0, 0x99847a),
];

fn band_table() -> &'static [(f32, Rgb)] {
    static TABLE: OnceLock<Vec<(f32, Rgb)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        BANDS_HEX
            .iter()
            .map(|&(deg, hex)| (deg, from_hex(hex)))
            .collect()
    })
}

#[inline]
fn band_color(lat_deg: f32) -> Rgb {
    let t = band_table();
    if lat_deg <= t[0].0 {
        return t[0].1;
    }
    for w in t.windows(2) {
        let (d0, c0) = w[0];
        let (d1, c1) = w[1];
        if lat_deg <= d1 {
            let f = smoothstep(d0, d1, lat_deg);
            return c0.lerp(c1, f);
        }
    }
    t[t.len() - 1].1
}

/// Full atmosphere albedo for a surface point.
fn surface(lat: f32, lon: f32) -> Rgb {
    // Meandering jets: wavy band boundaries that sweep with the rotation.
    let lat_band = lat
        + 0.130 * (lon * 2.0 + 0.9 * (lat * 5.0).sin()).sin()
        + 0.055 * (lon * 5.0 - 1.3 * (lat * 3.0).sin()).sin();
    let base = band_color(lat_band.to_degrees());

    // Fine ribbons and convective swirls.
    let mut k = 1.0_f32;
    k += 0.020 * (lat * 27.0 + 1.1 * (lon * 3.0).sin()).sin();
    k += 0.010 * (lat * 64.0 + 1.8 * (lon * 2.0 + 0.7).sin()).sin();
    k += 0.005 * (lat * 141.0).sin();
    k += 0.022 * (lon * 6.0 + 2.2 * (lat * 8.0).sin()).sin() * (lat * 2.2).cos();
    k += 0.012 * (lon * 13.0 - 1.5 * (lat * 5.0).sin()).sin() * (lat * 3.0).cos();
    k += 0.060 * (lon + 0.9 * (lat * 3.0).sin()).sin()
        + 0.035 * (lon * 2.0 - 0.6).sin() * (lat * 2.0).cos();
    // Large bright/dark sectors near the equator.
    k += 0.22 * (lon * 3.0 + 0.6).sin() * (1.0 - (lat / 0.5).powi(2)).max(0.0);

    let c = base.mul(k.clamp(0.65, 1.45));

    // Storms: bright white ovals and dark barges.  These are the features a
    // viewer can actually track as the planet turns.
    let white = 0.34 * oval(lat, lon, 0.40, 1.30, 0.28, 0.55)
        + 0.26 * oval(lat, lon, 0.28, -2.10, 0.20, 0.32)
        + 0.20 * oval(lat, lon, 0.60, 0.55, 0.16, 0.26)
        + 0.20 * oval(lat, lon, -0.38, -0.60, 0.22, 0.40)
        + 0.16 * oval(lat, lon, -0.56, 2.20, 0.16, 0.26)
        + 0.16 * oval(lat, lon, -0.12, 2.60, 0.15, 0.24);
    let dark = 0.28 * oval(lat, lon, -0.30, 0.90, 0.16, 0.34)
        + 0.17 * oval(lat, lon, 0.20, 3.05, 0.12, 0.22)
        + 0.11 * oval(lat, lon, 0.52, -1.55, 0.10, 0.18);

    const STORM_LIGHT: Rgb = Rgb::new(0.88, 0.80, 0.64);
    const STORM_DARK: Rgb = Rgb::new(0.17, 0.12, 0.075);

    c.lerp(STORM_LIGHT, white.clamp(0.0, 0.75))
        .lerp(STORM_DARK, dark.clamp(0.0, 0.65))
}

#[inline]
fn oval(lat: f32, lon: f32, lat0: f32, lon0: f32, sy: f32, sx: f32) -> f32 {
    let dl = wrap_pi(lon - lon0);
    let a = (lat - lat0) / sy;
    let b = dl / sx;
    let d2 = a * a + b * b;
    if d2 >= 1.0 {
        0.0
    } else {
        let f = 1.0 - d2;
        f * f
    }
}

#[inline]
fn wrap_pi(mut a: f32) -> f32 {
    while a > PI {
        a -= 2.0 * PI;
    }
    while a < -PI {
        a += 2.0 * PI;
    }
    a
}

// ---------------------------------------------------------------------------
// rings
// ---------------------------------------------------------------------------

struct RingSeg {
    r0: f32,
    r1: f32,
    alpha: f32,
    tone: Rgb,
}

const RINGS_HEX: &[(f32, f32, f32, u32)] = &[
    // D ring: faint, dusty
    (1.105, 1.236, 0.090, 0x4a4338),
    // C ring: translucent, grayish tan
    (1.236, 1.530, 0.40, 0xa89880),
    // B ring: the bright, dense one
    (1.530, 1.952, 0.98, 0xe9d9b6),
    // Cassini Division
    (1.952, 2.030, 0.045, 0x38332c),
    // A ring
    (2.030, 2.212, 0.86, 0xd8c7a6),
    // Encke Gap
    (2.212, 2.227, 0.035, 0x38332c),
    // outer A ring
    (2.227, 2.272, 0.72, 0xcdbb9a),
];

fn ring_table() -> &'static [RingSeg] {
    static TABLE: OnceLock<Vec<RingSeg>> = OnceLock::new();
    TABLE.get_or_init(|| {
        RINGS_HEX
            .iter()
            .map(|&(r0, r1, alpha, hex)| RingSeg {
                r0,
                r1,
                alpha,
                tone: from_hex(hex),
            })
            .collect()
    })
}

/// Opacity and albedo of the ring sheet at radius `r`.
fn ring_sample(r: f32) -> (f32, Rgb) {
    let segs = ring_table();
    let edge = 0.004;

    let mut alpha = 0.0_f32;
    let mut tone = Rgb::ZERO;
    let mut weight = 0.0_f32;

    for s in segs {
        let fade = smoothstep(s.r0 - edge, s.r0 + edge, r)
            * (1.0 - smoothstep(s.r1 - edge, s.r1 + edge, r));
        let a = s.alpha * fade;
        if a > alpha {
            alpha = a;
            tone = s.tone;
        }
        weight += fade;
    }

    if alpha <= 0.0 || weight <= 0.0 {
        return (alpha, tone);
    }

    // Ripples: unresolved ringlets, density waves and a few narrow gaps.
    let mut a = alpha;
    if r > 1.24 && r < 1.53 {
        // C ring: a broad wispy subdivision
        a *= 0.82 + 0.18 * (0.5 + 0.5 * (r * 12.0).sin());
    } else if r > 1.53 && r < 1.95 {
        // B ring: dense, with a couple of broad subdivisions
        a *= 0.98 + 0.02 * (r * 20.0).sin();
        a *= 1.0 - 0.16 * gauss(r, 1.642, 0.030) - 0.12 * gauss(r, 1.712, 0.024);
    } else if r > 2.03 && r < 2.212 {
        // A ring: mostly uniform, one extra gap
        a *= 1.0 - 0.55 * gauss(r, 2.102, 0.010);
    } else if r > 2.227 && r < 2.272 {
        a *= 0.96 + 0.04 * (r * 30.0).sin();
    }

    // Gentle brightness trend across the ringlets.
    let tone = tone.mul(0.97 + 0.03 * (r * 15.0).sin());

    (a.clamp(0.0, 1.0), tone)
}

#[inline]
fn gauss(x: f32, center: f32, width: f32) -> f32 {
    let d = (x - center) / width;
    (-d * d).exp()
}

// ---------------------------------------------------------------------------
// star field
// ---------------------------------------------------------------------------

fn star(col: usize, row: usize, time: f32) -> Rgb {
    let h = hash2(col as u32, row as u32);
    // roughly 18 in 1000 cells hold a star
    if h % 1000 >= 6 {
        return Rgb::ZERO;
    }
    let h2 = hash2(col as u32 ^ 0x9e37, row as u32 ^ 0x85eb);
    let phase = (h2 & 0xff) as f32 / 255.0 * PI * 2.0;
    let twinkle = 0.70 + 0.30 * (time * 1.7 + phase).sin();
    // Keep every star clearly brighter than the sky so none reads as a
    // black speck on a dark terminal theme.
    let base = 0.22 + 0.34 * ((h2 >> 8) & 0xff) as f32 / 255.0;
    let warm = (h2 >> 16) & 7 == 0;
    let c = if warm {
        // a few slightly warm stars
        Rgb::new(0.86, 0.82, 0.76)
    } else {
        // the rest are neutral white-grey
        Rgb::new(0.80, 0.80, 0.82)
    };
    c.mul(base * twinkle * 1.5)
}

#[inline]
fn hash2(x: u32, y: u32) -> u32 {
    let mut h = x.wrapping_mul(0x9e37_79b1) ^ y.wrapping_mul(0x85eb_ca77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e0 == e1 {
        return if x < e0 { 0.0 } else { 1.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Params {
        Params {
            tilt: 26.0_f32.to_radians(),
            azimuth: 18.0_f32.to_radians(),
            roll: 0.0,
            distance: 9.0,
            spin: 0.7,
            sun: V3::new(-0.58, 0.46, -0.67).norm(),
            time: 3.0,
            stars: true,
            ramp: CHARSETS[0].ramp,
        }
    }

    #[test]
    fn renders_at_any_size() {
        let p = params();
        for (c, r) in [(1, 1), (4, 3), (8, 6), (80, 24), (200, 60)] {
            let cells = render(c, r, &p, None);
            assert_eq!(cells.len(), c.max(4) * r.max(3));
        }
    }

    #[test]
    fn ring_bands_are_where_they_should_be() {
        // inside the planet / in the Encke gap there is nothing
        assert_eq!(ring_sample(0.8).0, 0.0);
        assert!(ring_sample(2.219).0 < 0.1);
        // the B ring is dense, the Cassini division is not
        assert!(ring_sample(1.75).0 > 0.8);
        assert!(ring_sample(1.99).0 < 0.12);
        // and it all ends at the outer edge
        assert_eq!(ring_sample(2.4).0, 0.0);
    }

    #[test]
    fn sphere_hit_lands_on_the_unit_sphere() {
        let o = V3::new(0.1, 0.2, -5.0);
        let d = V3::new(0.0, 0.0, 1.0);
        let (t, p) = sphere_hit(o, d).expect("center ray must hit");
        assert!(t > 0.0);
        assert!((p.dot(p).sqrt() - 1.0).abs() < 1e-4);
        // a ray pointing away misses
        assert!(sphere_hit(V3::new(3.0, 0.0, 0.0), d).is_none());
    }

    #[test]
    fn shadow_geometry() {
        let sun = V3::new(-0.58, 0.46, -0.67).norm();
        // a point on the night side, behind the planet from the sun
        let behind = V3::new(0.9, 0.0, 1.4);
        assert!(sphere_occlusion(behind, sun) > 0.5);
        // a point on the sunward side is lit
        assert_eq!(sphere_occlusion(V3::new(-1.4, 0.0, 0.6), sun), 0.0);
    }

    #[test]
    fn centre_of_disc_is_finite() {
        let p = V3::new(0.1706, 0.1908, -0.9683).norm();
        let d = p.mul(-1.0);
        let sun = V3::new(-0.18, 0.03, -0.983).norm();
        let c = shade_planet(p, d, 0.22, sun);
        assert!(c.r.is_finite() && c.g.is_finite() && c.b.is_finite());
        assert!(c.luma() > 0.2);
    }

    #[test]
    fn glyphs_cover_every_ramp() {
        for cs in CHARSETS {
            assert_eq!(cs.ramp[0], b' ');
            assert!(cs.ramp.len() >= 8, "{}", cs.name);
            assert_eq!(glyph_with_index(0, 0, 0, cs.ramp, None, 0).0, b' ');
            assert_eq!(
                glyph_with_index(255, 255, 255, cs.ramp, None, 0).0,
                *cs.ramp.last().unwrap()
            );
        }
        assert_eq!(CHARSETS[0].name, "classic");
    }
}
