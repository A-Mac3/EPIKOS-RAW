//! Make every Step 3 mask the installed models allow (subject, background, sky, skin,
//! eyes, hair, foreground) for a photo and write the preview plus each mask (as a
//! greyscale PNG and a red overlay) to disk, with timings.
//!
//! cargo run --release -p epikos-engine --example detect_masks -- <RAW> <OUT_DIR>

use std::path::PathBuf;
use std::time::Instant;

use epikos_engine::Engine;
use epikos_sidecar::Adjustments;

fn main() {
    let mut args = std::env::args().skip(1);
    let raw = PathBuf::from(args.next().expect("usage: detect_masks <RAW> <OUT_DIR>"));
    let out = PathBuf::from(args.next().expect("missing OUT_DIR"));
    std::fs::create_dir_all(&out).unwrap();
    let stem = raw.file_stem().unwrap().to_string_lossy().into_owned();

    let engine = Engine::default();
    println!("models in {}", engine.mask_models().dir);
    let info = engine.open(&raw).unwrap_or_else(|e| panic!("open failed: {e}"));
    let adj = Adjustments::default();
    let preview = engine.render_preview(&raw, &adj, 1024, 1024).unwrap();
    image::RgbaImage::from_raw(preview.width, preview.height, preview.rgba.clone())
        .unwrap()
        .save(out.join(format!("{stem}_preview.png")))
        .unwrap();
    println!("{} {} {}×{}", info.make, info.model, info.width, info.height);

    println!("lens profile: {:?}", info.lens_profile);
    for status in engine.mask_models().targets {
        let kind = status.target;
        if !status.available {
            println!("{:<10} not available ({})", kind.label(), status.source);
            continue;
        }
        for run in ["first", "warm"] {
            let t = Instant::now();
            let mask = engine
                .detect_mask(&raw, &adj, kind)
                .unwrap_or_else(|e| panic!("{kind:?}: {e}"));
            println!(
                "{:<10} {run:<5} {:>6} ms total  {:>5} ms inference  {}×{}  coverage {:.1}%",
                kind.label(),
                t.elapsed().as_millis(),
                mask.infer_ms,
                mask.width,
                mask.height,
                100.0 * mask.coverage()
            );
            if run == "first" {
                continue;
            }
            let name = kind.label().to_lowercase();
            image::GrayImage::from_raw(mask.width, mask.height, mask.alpha())
                .unwrap()
                .save(out.join(format!("{stem}_{name}.png")))
                .unwrap();
            // Red overlay on the preview, as the app shows it.
            if (mask.width, mask.height) == (preview.width, preview.height) {
                let mut rgba = preview.rgba.clone();
                for (px, &a) in rgba.chunks_mut(4).zip(&mask.alpha()) {
                    let t = 0.55 * a as f32 / 255.0;
                    px[0] = (px[0] as f32 * (1.0 - t) + 255.0 * t) as u8;
                    px[1] = (px[1] as f32 * (1.0 - t) + 40.0 * t) as u8;
                    px[2] = (px[2] as f32 * (1.0 - t) + 60.0 * t) as u8;
                }
                image::RgbaImage::from_raw(mask.width, mask.height, rgba)
                    .unwrap()
                    .save(out.join(format!("{stem}_{name}_overlay.png")))
                    .unwrap();
            }
        }
    }
}
