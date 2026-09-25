//! Render an engine preview and thumbnail to disk, with timings.
//!
//! cargo run --release -p epikos-engine --example render_preview -- <RAW> <OUT_DIR> [EV]

use std::path::PathBuf;
use std::time::Instant;

use epikos_engine::Engine;

fn main() {
    let mut args = std::env::args().skip(1);
    let raw = PathBuf::from(
        args.next()
            .expect("usage: render_preview <RAW> <OUT_DIR> [EV]"),
    );
    let out = PathBuf::from(args.next().expect("missing OUT_DIR"));
    let ev: f32 = args
        .next()
        .map(|v| v.parse().expect("EV must be a number"))
        .unwrap_or(0.0);
    std::fs::create_dir_all(&out).unwrap();
    let stem = raw.file_stem().unwrap().to_string_lossy().into_owned();
    let engine = Engine::default();

    let t = Instant::now();
    let info = engine
        .open(&raw)
        .unwrap_or_else(|e| panic!("open failed: {e}"));
    println!(
        "open        {:>6} ms  {} {} {}  {}×{}  mono={}  as-shot={:?}",
        t.elapsed().as_millis(),
        info.format,
        info.make,
        info.model,
        info.width,
        info.height,
        info.monochrome,
        info.as_shot
    );

    let mut adj = info.document.adjustments.clone();
    adj.exposure = ev;
    for pass in ["cold", "warm"] {
        let t = Instant::now();
        let d = engine.render_preview(&raw, &adj, 1600, 1200).unwrap();
        println!(
            "preview {pass} {:>6} ms  {}×{}",
            t.elapsed().as_millis(),
            d.width,
            d.height
        );
        if pass == "warm" {
            let img = image::RgbaImage::from_raw(d.width, d.height, d.rgba).unwrap();
            let dest = out.join(format!("{stem}-preview.png"));
            img.save(&dest).unwrap();
            println!("            wrote {}", dest.display());
        }
    }

    let t = Instant::now();
    let jpeg = engine.thumbnail_jpeg(&raw, 320).unwrap();
    let dest = out.join(format!("{stem}-thumb.jpg"));
    std::fs::write(&dest, jpeg).unwrap();
    println!(
        "thumbnail   {:>6} ms  wrote {}",
        t.elapsed().as_millis(),
        dest.display()
    );
}
