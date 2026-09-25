//! PRD Section 4 "3D Atmospheric Light Sculptor": virtual lights placed in the scene's
//! depth after capture, on upright scene-linear Rec.2020.
//!
//! The depth map turns the photo into a relief: each pixel sits at
//! `(x, y, depth · Z_RANGE)` in frame-width units, with a surface normal from the
//! (smoothed) depth gradient. Each light then adds
//! - **surface light**: Lambertian falloff towards the light with inverse-square-like
//!   attenuation, multiplied into the pixel so texture and colour survive;
//! - **rim light**: when the light is *behind* a surface, its edges (normals turned
//!   away from the camera) catch light, which is what a backlit subject looks like;
//! - **halo**: the light's glow in the air, visible only where nothing nearer than the
//!   light is in front of it, so a sun placed behind a tree glows around the tree.
//!
//! Without a depth model the scene is a flat plane at mid depth: lights still work
//! as soft 2D spots.

use epikos_core::ImageRgbF32;
use epikos_sidecar::VirtualLight;
use rayon::prelude::*;

use super::atmosphere::{percentile, tint};
use super::blur::box_blur;
use super::{long_side, smoothstep};

/// Depth of the scene relative to the frame's long side: the relief is this deep.
const Z_RANGE: f32 = 0.6;

pub(crate) fn apply_lights(rgb: &mut ImageRgbF32, lights: &[VirtualLight], depth: Option<&[f32]>) {
    let (w, h) = (rgb.width as usize, rgb.height as usize);
    let lights: Vec<&VirtualLight> = lights.iter().filter(|l| l.intensity > 0.0).collect();
    if lights.is_empty() || w < 4 || h < 4 {
        return;
    }
    let long = long_side(w, h);
    // z: 0 at the camera … 1 far, same scale as a light's `depth`.
    let z: Vec<f32> = match depth {
        Some(d) => d.par_iter().map(|v| 1.0 - v).collect(),
        None => vec![0.5; w * h],
    };
    let zs = box_blur(&z, w, h, (0.004 * long).round().max(1.0) as usize);
    let lum: Vec<f32> = (0..w * h)
        .into_par_iter()
        .map(|i| (0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i]).max(0.0))
        .collect();
    // Light levels follow the scene's own bright level, so they read alike at any exposure.
    let level = percentile(&lum, 0.95).max(0.05);

    let prepared: Vec<_> = lights
        .iter()
        .map(|l| {
            let pct = |v: f32| (v / 100.0).clamp(0.0, 1.0);
            (
                // Light position in frame-width units.
                [l.x.clamp(0.0, 1.0) * w as f32 / long, l.y.clamp(0.0, 1.0) * h as f32 / long, l.depth.clamp(0.0, 1.0) * Z_RANGE],
                2.5 * pct(l.intensity),
                0.05 + 0.6 * pct(l.reach),
                pct(l.halo),
                tint((l.warmth / 100.0).clamp(-1.0, 1.0)),
            )
        })
        .collect();

    // Per pixel: multiplicative surface gain and additive halo, per channel.
    let effects: Vec<([f32; 3], [f32; 3])> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let at = |xx: usize, yy: usize| zs[yy * w + xx];
            let (x0, x1) = (x.saturating_sub(1), (x + 1).min(w - 1));
            let (y0, y1) = (y.saturating_sub(1), (y + 1).min(h - 1));
            // dz per frame-width unit.
            let dzdu = (at(x1, y) - at(x0, y)) * Z_RANGE * long / (x1 - x0).max(1) as f32;
            let dzdv = (at(x, y1) - at(x, y0)) * Z_RANGE * long / (y1 - y0).max(1) as f32;
            // Normal of the surface z(u, v), facing the camera (−z).
            let n = normalize([dzdu, dzdv, -1.0]);
            let p = [(x as f32 + 0.5) / long, (y as f32 + 0.5) / long, zs[i] * Z_RANGE];

            let mut gain = [0.0f32; 3];
            let mut add = [0.0f32; 3];
            for &(l, strength, reach, halo, col) in &prepared {
                let d = [l[0] - p[0], l[1] - p[1], l[2] - p[2]];
                let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-4);
                let att = 1.0 / (1.0 + (dist / reach).powi(2));
                let diffuse = ((n[0] * d[0] + n[1] * d[1] + n[2] * d[2]) / dist).max(0.0);
                // Behind the surface: light wraps the silhouette edges.
                let behind = smoothstep(0.0, 0.08, l[2] - p[2]);
                let rim = (1.0 - n[2].abs()).powf(1.5) * behind;
                let surface = strength * att * (0.8 * diffuse + 1.4 * rim);
                // Halo in the air, hidden where something nearer than the light sits.
                let screen = ((l[0] - p[0]).powi(2) + (l[1] - p[1]).powi(2)).sqrt();
                let radius = 0.02 + 0.2 * reach * halo;
                let visible = smoothstep(-0.02, 0.02, p[2] - l[2]);
                let glow = halo * strength * 0.6 * level * (-(screen / radius).powi(2)).exp() * visible;
                for c in 0..3 {
                    gain[c] += surface * col[c];
                    add[c] += glow * col[c];
                }
            }
            (gain, add)
        })
        .collect();

    for (c, plane) in [&mut rgb.r, &mut rgb.g, &mut rgb.b].into_iter().enumerate() {
        plane
            .par_iter_mut()
            .zip(effects.par_iter())
            .for_each(|(v, (gain, add))| *v = *v * (1.0 + gain[c]) + add[c]);
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    v.map(|c| c / l)
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    fn grey(w: u32, h: u32, v: f32) -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(w, h, ColorSpace::LinearRec2020);
        for p in [&mut img.r, &mut img.g, &mut img.b] {
            p.fill(v);
        }
        img
    }

    #[test]
    fn a_light_brightens_what_is_near_it_most() {
        let mut img = grey(200, 100, 0.1);
        let light = VirtualLight { x: 0.25, y: 0.5, depth: 0.4, halo: 0.0, warmth: 0.0, ..Default::default() };
        apply_lights(&mut img, &[light], None);
        let (near, far) = (img.g[img.index(50, 50)], img.g[img.index(190, 50)]);
        assert!(near > 0.12 && near > far * 1.2, "near {near} far {far}");
    }

    #[test]
    fn a_light_behind_a_subject_rims_its_edges_and_hides_its_halo() {
        // A near square (depth map 1 = near) in front of a far background.
        let (w, h) = (200usize, 200usize);
        let depth: Vec<f32> = (0..w * h)
            .map(|i| if (60..140).contains(&(i % w)) && (60..140).contains(&(i / w)) { 1.0 } else { 0.0 })
            .collect();
        let light = VirtualLight { x: 0.5, y: 0.5, depth: 0.7, intensity: 100.0, halo: 100.0, warmth: 0.0, ..Default::default() };
        let mut img = grey(w as u32, h as u32, 0.1);
        apply_lights(&mut img, &[light], Some(&depth));
        let at = |x: usize, y: usize| img.g[y * w + x];
        // The square's centre faces the camera, not the light behind it: little change.
        assert!(at(100, 100) < 0.13, "centre {}", at(100, 100));
        // Its edge catches rim light.
        assert!(at(61, 100) > at(100, 100) * 1.3, "edge {} centre {}", at(61, 100), at(100, 100));
        // The halo shows around the square, in the background.
        assert!(at(50, 100) > 0.12, "no halo around the subject: {}", at(50, 100));
    }
}
