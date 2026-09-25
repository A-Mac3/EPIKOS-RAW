//! Render a RAW with each built-in style and with sample Step 4–5 settings, as PNGs
//! plus a timed full-resolution develop per look.
//!
//! cargo run --release -p epikos-engine --example render_looks -- <RAW> <OUT_DIR> [--full]

use std::path::PathBuf;
use std::time::Instant;

use epikos_engine::{styles, Engine};
use epikos_sidecar::Adjustments;

fn main() {
    let mut args = std::env::args().skip(1);
    let raw = PathBuf::from(args.next().expect("usage: render_looks <RAW> <OUT_DIR> [--full]"));
    let out = PathBuf::from(args.next().expect("missing OUT_DIR"));
    let full = args.next().as_deref() == Some("--full");
    std::fs::create_dir_all(&out).unwrap();
    let stem = raw.file_stem().unwrap().to_string_lossy().into_owned();

    let engine = Engine::default();
    let info = engine.open(&raw).unwrap_or_else(|e| panic!("open failed: {e}"));
    let base = info.document.adjustments.clone();

    let mut looks: Vec<(String, Adjustments)> = vec![("original".into(), base.clone())];
    for s in styles() {
        let mut a = base.clone();
        a.style.id = s.id.into();
        a.style.amount = 100.0;
        a.style.skin_protection = s.skin_protection;
        looks.push((s.id.into(), a));
    }
    let mut manual = base.clone();
    manual.texture.clarity = 30.0;
    manual.texture.micro_texture = 40.0;
    manual.texture.blemish_smoothing = 60.0;
    manual.texture.specular_balance = 50.0;
    manual.color.hsl.blue.saturation = 30.0;
    manual.color.hsl.green.hue = -30.0;
    manual.color.wheels.shadows = epikos_sidecar::ColorWheel { hue: 200.0, amount: 20.0, luminance: -10.0 };
    manual.color.wheels.highlights = epikos_sidecar::ColorWheel { hue: 40.0, amount: 20.0, luminance: 0.0 };
    manual.color.skin_protection = 80.0;
    looks.push(("manual".into(), manual));
    let mut step6 = base.clone();
    step6.atmosphere.glow = 35.0;
    step6.atmosphere.fog = 55.0;
    step6.atmosphere.fog_start = 20.0;
    step6.atmosphere.fog_warmth = 20.0;
    step6.atmosphere.shafts = 70.0;
    looks.push(("step6".into(), step6));
    let mut step7 = base.clone();
    step7.curves.rgb.darks = -25.0;
    step7.curves.rgb.lights = 20.0;
    step7.curves.rgb.black = 35.0;
    step7.curves.blue.shadows = 15.0;
    step7.split_toning.highlight_hue = 38.0;
    step7.split_toning.highlight_saturation = 35.0;
    step7.split_toning.shadow_hue = 190.0;
    step7.split_toning.shadow_saturation = 40.0;
    looks.push(("step7".into(), step7));

    // Depth map (Step 6), if the model is installed.
    if engine.mask_models().depth.available {
        let t = Instant::now();
        let d = engine.depth(&raw, &base).unwrap();
        println!("depth {}×{} in {} ms ({} ms inference)", d.width, d.height, t.elapsed().as_millis(), d.infer_ms);
        let px: Vec<u8> = d.depth.iter().map(|v| (v * 255.0).round() as u8).collect();
        image::GrayImage::from_raw(d.width, d.height, px)
            .unwrap()
            .save(out.join(format!("{stem}_depth.png")))
            .unwrap();
    }

    // Skin map of the unstyled develop, for checking the detector.
    {
        let decoded = epikos_decode::decode_file(&raw).unwrap();
        let block = epikos_pipeline::block_for_size(&decoded.mosaic, 1600, 1600);
        let rgb = epikos_pipeline::develop_rgb(
            epikos_pipeline::bin_mosaic(&decoded.mosaic, block),
            &decoded.profile,
            &base,
        )
        .unwrap();
        let skin = epikos_pipeline::skin_likelihood(&rgb);
        // EPIKOS_PROBE="x,y;x,y": print linear RGB and skin likelihood at those pixels.
        if let Ok(probe) = std::env::var("EPIKOS_PROBE") {
            for pt in probe.split(';') {
                let (x, y) = pt.split_once(',').unwrap();
                let (x, y): (u32, u32) = (x.parse().unwrap(), y.parse().unwrap());
                let i = rgb.index(x, y);
                println!(
                    "probe {x},{y}: rgb {:.4} {:.4} {:.4} skin {:.2}",
                    rgb.r[i], rgb.g[i], rgb.b[i], skin[i]
                );
            }
        }
        let px: Vec<u8> = skin.iter().map(|v| (v * 255.0).round() as u8).collect();
        image::GrayImage::from_raw(rgb.width, rgb.height, px)
            .unwrap()
            .save(out.join(format!("{stem}_skin.png")))
            .unwrap();
    }

    for (name, adj) in &looks {
        let t = Instant::now();
        let p = engine.render_preview(&raw, adj, 1600, 1600).unwrap();
        let preview_ms = t.elapsed().as_millis();
        image::RgbaImage::from_raw(p.width, p.height, p.rgba)
            .unwrap()
            .save(out.join(format!("{stem}_{name}.png")))
            .unwrap();
        let mut line = format!("{name:<24} preview {:>4}×{:<4} {preview_ms:>5} ms", p.width, p.height);
        if full {
            let dest = out.join(format!("{stem}_{name}.tif"));
            let r = engine
                .export(&raw, adj, &dest, Default::default())
                .unwrap();
            line += &format!("   full {}×{} develop {} ms", r.width, r.height, r.develop_ms);
        }
        println!("{line}");
    }
}
