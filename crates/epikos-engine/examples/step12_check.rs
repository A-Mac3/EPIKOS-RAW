//! Step 1–2 check: render a photo with the lens profile off and on, and print the
//! auto-tone and auto-upright suggestions.
//!
//! cargo run --release -p epikos-engine --example step12_check -- <OUT_DIR> <FILE>…

use std::path::PathBuf;

use epikos_engine::Engine;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().expect("usage: step12_check <OUT_DIR> <FILE>…"));
    std::fs::create_dir_all(&out).unwrap();
    let engine = Engine::default();
    for file in args {
        let path = PathBuf::from(&file);
        let info = engine.open(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let mut adj = info.document.adjustments.clone();
        for on in [false, true] {
            adj.lens.profile = on;
            let t = std::time::Instant::now();
            let d = engine.render_preview(&path, &adj, 1200, 1200).unwrap();
            let ms = t.elapsed().as_millis();
            let name = out.join(format!("{stem}_profile_{}.png", if on { "on" } else { "off" }));
            image::RgbaImage::from_raw(d.width, d.height, d.rgba).unwrap().save(&name).unwrap();
            println!("{stem}: profile {on} → {} ({ms} ms)", name.display());
        }
        let (ev, tone) = engine.auto_tone(&path, &adj).unwrap();
        let (rotation, vertical) = engine.auto_upright(&path, &adj).unwrap();
        adj.lens.rotation = rotation;
        adj.lens.vertical = vertical;
        adj.exposure = ev;
        adj.tone = tone;
        let d = engine.render_preview(&path, &adj, 1200, 1200).unwrap();
        image::RgbaImage::from_raw(d.width, d.height, d.rgba).unwrap().save(out.join(format!("{stem}_auto.png"))).unwrap();
        println!(
            "{stem}: {} | lens profile {:?} | auto tone {ev:+.2} EV {tone:?} | auto upright {rotation:+.2}° vertical {vertical:+.0}",
            info.format, info.lens_profile
        );
    }
}
