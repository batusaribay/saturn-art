//! Tiny linear-light color type.
//!
//! Shading is done in linear RGB and only converted to sRGB when the glyph is
//! emitted, which keeps gradients and shadowed regions physically plausible.

use std::ops::{Add, Mul};

#[derive(Clone, Copy, Debug, Default)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Rgb {
    pub const ZERO: Rgb = Rgb {
        r: 0.0,
        g: 0.0,
        b: 0.0,
    };

    #[inline]
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }

    #[inline]
    pub fn mul(self, k: f32) -> Self {
        Self::new(self.r * k, self.g * k, self.b * k)
    }

    /// Component-wise product (albedo * light).
    #[inline]
    pub fn mul_rgb(self, o: Rgb) -> Self {
        Self::new(self.r * o.r, self.g * o.g, self.b * o.b)
    }

    #[inline]
    pub fn add(self, o: Rgb) -> Self {
        Self::new(self.r + o.r, self.g + o.g, self.b + o.b)
    }

    #[inline]
    pub fn lerp(self, o: Rgb, t: f32) -> Self {
        Self::new(
            self.r + (o.r - self.r) * t,
            self.g + (o.g - self.g) * t,
            self.b + (o.b - self.b) * t,
        )
    }

    /// Composite `src` with `alpha` over `self`.
    #[inline]
    pub fn over(self, src: Rgb, alpha: f32) -> Self {
        let a = alpha.clamp(0.0, 1.0);
        Self::new(
            src.r * a + self.r * (1.0 - a),
            src.g * a + self.g * (1.0 - a),
            src.b * a + self.b * (1.0 - a),
        )
    }

    #[inline]
    pub fn luma(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }

    #[inline]
    pub fn to_srgb8(self) -> (u8, u8, u8) {
        (
            (linear_to_srgb(self.r) * 255.0 + 0.5) as u8,
            (linear_to_srgb(self.g) * 255.0 + 0.5) as u8,
            (linear_to_srgb(self.b) * 255.0 + 0.5) as u8,
        )
    }
}

impl Add for Rgb {
    type Output = Rgb;
    #[inline]
    fn add(self, o: Rgb) -> Rgb {
        Rgb::new(self.r + o.r, self.g + o.g, self.b + o.b)
    }
}

impl Mul<f32> for Rgb {
    type Output = Rgb;
    #[inline]
    fn mul(self, k: f32) -> Rgb {
        Rgb::new(self.r * k, self.g * k, self.b * k)
    }
}

#[inline]
pub fn srgb_to_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[inline]
pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Build a linear RGB color from a 0xRRGGBB sRGB literal.
pub fn from_hex(h: u32) -> Rgb {
    let r = ((h >> 16) & 0xff) as f32 / 255.0;
    let g = ((h >> 8) & 0xff) as f32 / 255.0;
    let b = (h & 0xff) as f32 / 255.0;
    Rgb::new(srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composites_source_and_destination_by_alpha() {
        let background = Rgb::new(0.2, 0.4, 0.6);
        let source = Rgb::new(0.8, 0.2, 0.0);
        let result = background.over(source, 0.25);
        assert!((result.r - 0.35).abs() < 1e-6);
        assert!((result.g - 0.35).abs() < 1e-6);
        assert!((result.b - 0.45).abs() < 1e-6);
        assert_eq!(background.over(source, 0.0).r, background.r);
        assert_eq!(background.over(source, 1.0).r, source.r);
    }
}
