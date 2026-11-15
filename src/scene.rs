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

use crate::color::{from_hex, Rgb};

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

/// Distance of the imaginary orthographic camera.  Only used to keep the
/// ray origin comfortably in front of the geometry.
const CAM_D: f32 = 9.0;

/// Character cells are about half as wide as they are tall on screen.
const CELL_ASPECT: f32 = 0.5;

/// Brightness ramp, dark to bright.
const RAMP: &[u8] = b" .:-=+*#%@";

// ---------------------------------------------------------------------------
// output
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
pub struct Cell {
    pub ch: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub struct Params {
    /// Tilt of the ring plane toward the viewer, in radians.
    pub tilt: f32,
    /// Planet rotation angle, in radians.
    pub spin: f32,
    /// Unit vector pointing from the planet toward the sun.
    pub sun: V3,
    /// Time in seconds, used only for the star twinkle.
    pub time: f32,
    pub stars: bool,
}

// ---------------------------------------------------------------------------
// main renderer
// ---------------------------------------------------------------------------

pub fn render(cols: usize, rows: usize, p: &Params) -> Vec<Cell> {
    let cols = cols.max(4);
    let rows = rows.max(3);

    let (st, ct) = p.tilt.sin_cos();

    // Frame the ring system: half the vertical extent of the view, in planet
    // radii, chosen so the widest part of the rings fits both ways.
    let half_h = (RING_OUTER * 1.07 * 2.0 * rows as f32 / cols as f32)
        .max(1.30)
        .max(RING_OUTER * st.abs() * 1.06 + 0.03);
    let half_w = half_h * cols as f32 / rows as f32 * CELL_ASPECT;
    // Camera space -> world space is a rotation about x by +tilt.
    let dir = V3::new(0.0, -st, ct);
    let sun = p.sun.norm();

    let mut cells = vec![Cell::default(); cols * rows];

    // 2x2 supersampling: rings are only a cell or two thick at the ansae, so
    // single-sample rays would make them flicker and drop out.
    const SUB: [(f32, f32); 4] = [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];

    for j in 0..rows {
        for i in 0..cols {
            let bg = if p.stars {
                star(i, j, p.time)
            } else {
                Rgb::ZERO
            };

            let mut acc = Rgb::ZERO;
            let mut covered = 0.0_f32;
            for (ox, oy) in SUB {
                let x = ((i as f32 + 0.5 + ox) / cols as f32 * 2.0 - 1.0) * half_w;
                let y = (1.0 - (j as f32 + 0.5 + oy) / rows as f32 * 2.0) * half_h;

                // Orthographic ray, tilted into world space.
                let o = V3::new(x, y * ct + CAM_D * st, y * st - CAM_D * ct);

                let (c, cov) = shade_ray(o, dir, p.spin, sun, bg);
                acc = acc.add(c);
                covered += cov;
            }

            let col = acc.mul(0.25);
            covered *= 0.25;

            let (r, g, b) = col.to_srgb8();
            let is_star = bg.luma() > 0.0 && covered < 0.4;
            let ch = if is_star {
                if (r as u32 + g as u32 + b as u32) > 270 {
                    b'+'
                } else {
                    b'.'
                }
            } else {
                glyph(r, g, b)
            };

            cells[j * cols + i] = Cell { ch, r, g, b };
        }
    }

    cells
}

/// Trace one ray, compositing what it hits over the background `bg`.
/// Also returns how much of the ray was covered by Saturn (0..1).
#[inline]
fn shade_ray(o: V3, dir: V3, spin: f32, sun: V3, bg: Rgb) -> (Rgb, f32) {
    let sphere = sphere_hit(o, dir);
    let ring = ring_hit(o, dir, sun);

    match sphere {
        Some((t_s, pos)) => {
            let surface = shade_planet(pos, dir, spin, sun);
            // The planet is opaque.  A ring hit in front of it is blended on
            // top; a hit behind it is hidden.
            let col = match ring {
                Some((t_r, alpha, rc)) if t_r < t_s => surface.over(rc, alpha),
                _ => surface,
            };
            (col, 1.0)
        }
        None => match ring {
            Some((_, alpha, rc)) => (bg.over(rc, alpha), alpha),
            None => (bg, 0.0),
        },
    }
}

/// Pick an ASCII glyph for an sRGB color.
#[inline]
fn glyph(r: u8, g: u8, b: u8) -> u8 {
    let lum = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0;
    let v = lum.powf(1.00) * (RAMP.len() - 1) as f32;
    RAMP[(v.round() as usize).min(RAMP.len() - 1)]
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
fn ring_hit(o: V3, d: V3, sun: V3) -> Option<(f32, f32, Rgb)> {
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
    let light = 0.54 + 0.90 * ndl;

    // The planet's shadow falls across the far side of the rings.  It is deep
    // but never pure black: scattered light still reaches the ring particles.
    let shadow = 1.0 - 0.62 * sphere_occlusion(q, sun);

    // Slow density waves in the ring plane.
    let ang = q.z.atan2(q.x);
    let wave = 0.93 + 0.07 * (ang * 22.0 + radius * 26.0).sin();

    Some((t, alpha, tone.mul(light * shadow * wave)))
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
    let ndl = n.dot(sun);
    let diff = smoothstep(-0.02, 0.11, ndl);
    let lambert = 0.34 + 0.66 * ndl.max(0.0);
    let mut light = (0.048 + 0.952 * diff) * lambert * 0.90;

    // Rings throw a shadow band onto the planet.
    let shadow = ring_shadow_on_planet(p, sun);
    light *= 1.0 - 0.72 * shadow;

    // Limb darkening + a pale atmospheric rim on the sunlit edge.
    let ndv = n.dot(d.mul(-1.0)).max(0.0);
    let limb = 0.68 + 0.32 * ndv.powf(0.45);
    let rim = (1.0 - ndv).powf(2.6) * 0.13 * smoothstep(0.0, 0.10, ndl);

    // A whisper of desaturation keeps the butterscotch from going orange.
    let grey = Rgb::new(albedo.luma(), albedo.luma(), albedo.luma());
    let albedo = albedo.lerp(grey, 0.10);

    albedo
        .mul(light * limb)
        .add(Rgb::new(0.86, 0.79, 0.63).mul(rim))
}

// ---------------------------------------------------------------------------
// Saturn's atmosphere: latitude bands with real-ish colors
// ---------------------------------------------------------------------------

/// Latitude (degrees) -> albedo, from the south pole (-90) to the north (90).
/// Pale butterscotch belts, cream equatorial zone, bluish-gray polar hood.
const BANDS_HEX: &[(f32, u32)] = &[
    (-90.0, 0x6f6557),
    (-82.0, 0x8e8270),
    (-74.0, 0xab9f86),
    (-66.0, 0xc4b79a),
    (-58.0, 0xd3c49f),
    (-50.0, 0xc6b992),
    (-42.0, 0xe4d5ae),
    (-34.0, 0xe8dbb8),
    (-27.0, 0xd7bd8a),
    (-20.0, 0xe0c998),
    (-13.0, 0xeee1bc),
    (-6.0, 0xf7edcb),
    (1.0, 0xf6ecc9),
    (8.0, 0xebdcb2),
    (14.0, 0xd6b884),
    (20.0, 0xdfc795),
    (27.0, 0xebe0b8),
    (34.0, 0xd3bd94),
    (41.0, 0xc9bb92),
    (48.0, 0xd9cca6),
    (55.0, 0xcfc3a2),
    (62.0, 0xcbbfa2),
    (70.0, 0xc1b9a4),
    (77.0, 0xb6b4a8),
    (83.0, 0xa8ada5),
    (88.0, 0x9ea7a3),
    (90.0, 0x99a3a0),
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
    // Saturn's jet streams meander, so the band boundaries are not perfect
    // circles.  This latitude ripple is what makes the rotation legible.
    let lat_band = lat
        + 0.085 * (lon * 2.0 + 0.9 * (lat * 5.0).sin()).sin()
        + 0.030 * (lon * 5.0 - 1.3 * (lat * 3.0).sin()).sin();
    let c = band_color(lat_band.to_degrees());

    // Fine zonal banding: Saturn's belts are full of thin ribbons.
    let mut k = 1.0_f32;
    k += 0.020 * (lat * 27.0 + 1.1 * (lon * 3.0).sin()).sin();
    k += 0.010 * (lat * 64.0 + 1.8 * (lon * 2.0 + 0.7).sin()).sin();
    k += 0.005 * (lat * 141.0).sin();

    // Convective swirls drifting along the bands.
    k += 0.020 * (lon * 6.0 + 2.2 * (lat * 8.0).sin()).sin() * (lat * 2.2).cos();
    k += 0.011 * (lon * 13.0 - 1.5 * (lat * 5.0).sin()).sin() * (lat * 3.0).cos();

    // Storm ovals and dark barges drifting with the rotation.  The big white
    // oval is a Great White Spot-style outbreak, ~30 degrees of longitude.
    k += 0.22 * oval(lat, lon, 0.40, 1.30, 0.22, 0.46);
    k += 0.16 * oval(lat, lon, 0.28, -2.10, 0.16, 0.28);
    k += 0.13 * oval(lat, lon, 0.62, 0.55, 0.13, 0.22);
    k += 0.12 * oval(lat, lon, -0.40, -0.60, 0.17, 0.34);
    k += 0.10 * oval(lat, lon, -0.56, 2.20, 0.12, 0.22);
    k += 0.10 * oval(lat, lon, -0.12, 2.60, 0.11, 0.20);
    k -= 0.09 * oval(lat, lon, -0.31, 0.90, 0.050, 0.22);
    k -= 0.08 * oval(lat, lon, 0.20, 3.05, 0.030, 0.10);
    k -= 0.06 * oval(lat, lon, 0.52, -1.55, 0.025, 0.08);

    c.mul(k.clamp(0.62, 1.55))
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
    (1.105, 1.236, 0.100, 0x5c5347),
    // C ring: translucent, grayish tan
    (1.236, 1.530, 0.55, 0xa9a291),
    // B ring: the bright, dense one
    (1.530, 1.952, 0.95, 0xeee5d2),
    // Cassini Division
    (1.952, 2.030, 0.045, 0x4e4941),
    // A ring
    (2.030, 2.212, 0.80, 0xdcd3bf),
    // Encke Gap
    (2.212, 2.227, 0.035, 0x4e4941),
    // outer A ring
    (2.227, 2.272, 0.66, 0xd6cdb8),
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
        // C ring: wispy, structured
        a *= 0.62 + 0.38 * (0.5 + 0.5 * (r * 118.0).sin());
    } else if r > 1.53 && r < 1.95 {
        // B ring: dense with fine ringlets
        a *= 0.93 + 0.07 * (r * 190.0).sin() * (r * 61.0).sin();
        // a few darker subdivisions
        a *= 1.0 - 0.22 * gauss(r, 1.642, 0.012) - 0.16 * gauss(r, 1.712, 0.009);
    } else if r > 2.03 && r < 2.212 {
        // A ring: mostly uniform, one extra gap
        a *= 1.0 - 0.55 * gauss(r, 2.102, 0.006);
        a *= 0.95 + 0.05 * (r * 260.0).sin();
    } else if r > 2.227 && r < 2.272 {
        a *= 0.90 + 0.10 * (r * 300.0).sin();
    }

    // Slight brightness variation across the ringlets.
    let tone = tone.mul(0.94 + 0.06 * (r * 47.0).sin());

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
    // roughly 4 in 1000 cells hold a star
    if h % 1000 >= 4 {
        return Rgb::ZERO;
    }
    let h2 = hash2(col as u32 ^ 0x9e37, row as u32 ^ 0x85eb);
    let phase = (h2 & 0xff) as f32 / 255.0 * PI * 2.0;
    let twinkle = 0.62 + 0.38 * (time * 1.7 + phase).sin();
    let base = 0.10 + 0.16 * ((h2 >> 8) & 0xff) as f32 / 255.0;
    let warm = (h2 >> 16) & 3 == 0;
    let c = if warm {
        Rgb::new(0.52, 0.44, 0.34)
    } else {
        Rgb::new(0.38, 0.48, 0.60)
    };
    c.mul(base * twinkle * 1.6)
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
            spin: 0.7,
            sun: V3::new(-0.58, 0.46, -0.67).norm(),
            time: 3.0,
            stars: true,
        }
    }

    #[test]
    fn renders_at_any_size() {
        let p = params();
        for (c, r) in [(1, 1), (4, 3), (8, 6), (80, 24), (200, 60)] {
            let cells = render(c, r, &p);
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
    fn glyphs_cover_the_ramp() {
        assert_eq!(glyph(0, 0, 0), b' ');
        assert_eq!(glyph(255, 255, 255), b'@');
        assert!(RAMP.len() >= 8);
    }
}
