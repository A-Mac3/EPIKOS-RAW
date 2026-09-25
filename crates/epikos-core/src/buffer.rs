use crate::color::ColorSpace;
use crate::error::{Error, Result};
use crate::profile::CfaPattern;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pixel {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Pixel {
    pub fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }
}

/// Planar RGB image, 32-bit float per channel, scene-referred (values may exceed 1.0).
#[derive(Debug, Clone)]
pub struct ImageRgbF32 {
    pub width: u32,
    pub height: u32,
    pub r: Vec<f32>,
    pub g: Vec<f32>,
    pub b: Vec<f32>,
    pub space: ColorSpace,
}

impl ImageRgbF32 {
    pub fn new(width: u32, height: u32, space: ColorSpace) -> Self {
        let n = (width as usize) * (height as usize);
        Self {
            width,
            height,
            r: vec![0.0; n],
            g: vec![0.0; n],
            b: vec![0.0; n],
            space,
        }
    }

    pub fn len(&self) -> usize {
        (self.width as usize) * (self.height as usize)
    }

    pub fn index(&self, x: u32, y: u32) -> usize {
        (y as usize) * (self.width as usize) + (x as usize)
    }

    pub fn get(&self, x: u32, y: u32) -> Pixel {
        let i = self.index(x, y);
        Pixel::new(self.r[i], self.g[i], self.b[i])
    }

    pub fn set(&mut self, x: u32, y: u32, px: Pixel) {
        let i = self.index(x, y);
        self.r[i] = px.r;
        self.g[i] = px.g;
        self.b[i] = px.b;
    }

    pub fn sample_bilinear(&self, x: f32, y: f32) -> Pixel {
        let w = self.width as i32;
        let h = self.height as i32;
        if w <= 1 || h <= 1 {
            return self.get(0, 0);
        }
        let x = x.clamp(0.0, (w - 1) as f32);
        let y = y.clamp(0.0, (h - 1) as f32);
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let x1 = (x0 + 1).min(w - 1);
        let y1 = (y0 + 1).min(h - 1);
        let tx = x - x0 as f32;
        let ty = y - y0 as f32;
        let p00 = self.get(x0 as u32, y0 as u32);
        let p10 = self.get(x1 as u32, y0 as u32);
        let p01 = self.get(x0 as u32, y1 as u32);
        let p11 = self.get(x1 as u32, y1 as u32);
        Pixel {
            r: lerp(lerp(p00.r, p10.r, tx), lerp(p01.r, p11.r, tx), ty),
            g: lerp(lerp(p00.g, p10.g, tx), lerp(p01.g, p11.g, tx), ty),
            b: lerp(lerp(p00.b, p10.b, tx), lerp(p01.b, p11.b, tx), ty),
        }
    }

    pub fn sample_channel_bilinear(&self, channel: u8, x: f32, y: f32) -> f32 {
        let plane = match channel {
            0 => &self.r,
            2 => &self.b,
            _ => &self.g,
        };
        sample_plane(plane, self.width, self.height, x, y)
    }

    pub fn validate(&self) -> Result<()> {
        let n = self.len();
        if self.r.len() != n || self.g.len() != n || self.b.len() != n {
            return Err(Error::InvalidImage {
                reason: "planar channel length mismatch".into(),
            });
        }
        Ok(())
    }
}

/// Single-channel mosaiced sensor (CFA) or packed linear samples.
#[derive(Debug, Clone)]
pub struct MosaicF32 {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
    pub samples_per_pixel: u32,
    pub cfa: Option<CfaPattern>,
}

impl MosaicF32 {
    pub fn index(&self, x: u32, y: u32) -> usize {
        ((y as usize) * (self.width as usize) + (x as usize)) * self.samples_per_pixel as usize
    }

    pub fn get(&self, x: u32, y: u32) -> f32 {
        self.data[self.index(x, y)]
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn sample_plane(plane: &[f32], width: u32, height: u32, x: f32, y: f32) -> f32 {
    let w = width as i32;
    let h = height as i32;
    if w <= 1 || h <= 1 {
        return plane.first().copied().unwrap_or(0.0);
    }
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let i = |xx: i32, yy: i32| (yy as usize) * (width as usize) + (xx as usize);
    let p00 = plane[i(x0, y0)];
    let p10 = plane[i(x1, y0)];
    let p01 = plane[i(x0, y1)];
    let p11 = plane[i(x1, y1)];
    lerp(lerp(p00, p10, tx), lerp(p01, p11, tx), ty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bilinear_identity_at_texels() {
        let mut img = ImageRgbF32::new(2, 2, ColorSpace::CameraRgb);
        img.set(0, 0, Pixel::new(1.0, 0.0, 0.0));
        img.set(1, 0, Pixel::new(0.0, 1.0, 0.0));
        img.set(0, 1, Pixel::new(0.0, 0.0, 1.0));
        img.set(1, 1, Pixel::new(1.0, 1.0, 1.0));
        let p = img.sample_bilinear(0.0, 0.0);
        assert!((p.r - 1.0).abs() < 1e-6);
        let mid = img.sample_bilinear(0.5, 0.0);
        assert!((mid.r - 0.5).abs() < 1e-5);
        assert!((mid.g - 0.5).abs() < 1e-5);
    }
}
