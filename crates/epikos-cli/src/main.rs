use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use epikos_core::{ImageRgbF32, Result};
use epikos_decode::decode_file;
use epikos_engine::{Engine, ExportFormat, ExportOptions, OutputSpace};
use epikos_pipeline::develop;
use epikos_sidecar::{
    load_json, save_json, save_xmp, sidecar_json_path, sidecar_xmp_path, DevelopDocument, SourceRef,
};

#[derive(Parser)]
#[command(name = "epikos", about = "EPIKOS native RAW / DNG engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print sensor profile for a RAW/DNG file.
    Inspect { path: PathBuf },
    /// Write empty JSON + XMP sidecars next to the RAW (never touches the original).
    Sidecar { path: PathBuf },
    /// Develop to a 32-bit linear Rec.2020 PFM using the sidecar (or defaults).
    Develop {
        path: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long)]
        sidecar: Option<PathBuf>,
    },
    /// Scene analysis of one photo (genre, lighting, skin, palette) as JSON.
    Analyze { path: PathBuf },
    /// Story-arc groups of the RAW files in a folder, with hero frames, as JSON.
    Story { dir: PathBuf },
    /// The AI Mentor's reading of a photo (insights and a recommended start), as JSON.
    Mentor { path: PathBuf },
    /// The built-in styles, as JSON.
    Styles,
    /// Export a full-resolution 16-bit TIFF, layered PSD or enhanced DNG (by extension).
    Export {
        path: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        /// Output colour space.
        #[arg(long, value_enum, default_value = "srgb")]
        space: Space,
        /// Exposure override in EV (otherwise the sidecar value).
        #[arg(long)]
        ev: Option<f32>,
        /// Leave GPS location out of the exported metadata.
        #[arg(long)]
        no_location: bool,
        /// Add the subject, sky, skin, eyes and hair masks as named alpha channels (Photoshop).
        #[arg(long)]
        masks: bool,
        /// Add the depth map as an alpha channel.
        #[arg(long)]
        depth: bool,
        /// Downsize so the long edge is at most this many pixels.
        #[arg(long)]
        long_edge: Option<u32>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Space {
    Srgb,
    P3,
    Prophoto,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Inspect { path } => inspect(&path),
        Commands::Sidecar { path } => init_sidecar(&path),
        Commands::Develop { path, out, sidecar } => develop_cmd(&path, out, sidecar),
        Commands::Analyze { path } => {
            let engine = Engine::new(1);
            let adjustments = engine.open(&path)?.document.adjustments;
            print_json(&engine.analyze(&path, &adjustments)?)
        }
        Commands::Story { dir } => print_json(&Engine::new(1).story_arc(&dir)?),
        Commands::Styles => print_json(&epikos_engine::styles()),
        Commands::Mentor { path } => {
            let engine = Engine::new(1);
            let adjustments = engine.open(&path)?.document.adjustments;
            let report = engine.mentor(&path, &adjustments)?;
            let feedback = engine.critique(&path, &report.recommended)?;
            print_json(&serde_json::json!({ "report": report, "feedbackOnRecommended": feedback }))
        }
        Commands::Export {
            path,
            out,
            space,
            ev,
            no_location,
            masks,
            depth,
            long_edge,
        } => {
            let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let options = ExportOptions {
                // From the output name: .psd (layered), .dng (enhanced), else TIFF.
                format: match ext.as_str() {
                    "psd" => ExportFormat::Psd,
                    "dng" => ExportFormat::Dng,
                    _ => ExportFormat::Tiff,
                },
                color_space: match space {
                    Space::Srgb => OutputSpace::Srgb,
                    Space::P3 => OutputSpace::DisplayP3,
                    Space::Prophoto => OutputSpace::ProPhoto,
                },
                include_location: !no_location,
                ai_masks: masks,
                depth_channel: depth,
                long_edge,
            };
            export_cmd(&path, &out, ev, options)
        }
    }
}

fn inspect(path: &Path) -> Result<()> {
    let decoded = decode_file(path)?;
    let p = &decoded.profile;
    println!("file        {}", decoded.source_path);
    println!("sha256      {}", decoded.source_sha256);
    println!("format      {}", p.format.label());
    println!("camera      {} {}", p.clean_make, p.clean_model);
    println!(
        "sensor      {}×{}  {} bps  {} spp",
        p.width, p.height, p.bits_per_sample, p.samples_per_pixel
    );
    println!("layout      {:?}", p.layout);
    println!("as-shot WB  {:?}", p.as_shot_wb);
    Ok(())
}

fn source_ref(decoded: &epikos_decode::DecodedRaw) -> SourceRef {
    SourceRef {
        path: decoded.source_path.clone(),
        sha256: decoded.source_sha256.clone(),
        format: decoded.profile.format.label().to_string(),
        make: decoded.profile.clean_make.clone(),
        model: decoded.profile.clean_model.clone(),
    }
}

fn init_sidecar(path: &Path) -> Result<()> {
    let decoded = decode_file(path)?;
    let doc = DevelopDocument::new(source_ref(&decoded));
    let json = sidecar_json_path(path);
    let xmp = sidecar_xmp_path(path);
    save_json(&json, &doc)?;
    save_xmp(&xmp, &doc)?;
    println!("wrote {}", json.display());
    println!("wrote {}", xmp.display());
    Ok(())
}

fn develop_cmd(path: &Path, out: Option<PathBuf>, sidecar: Option<PathBuf>) -> Result<()> {
    let decoded = decode_file(path)?;
    let json_path = sidecar.unwrap_or_else(|| sidecar_json_path(path));
    let doc = if json_path.exists() {
        load_json(&json_path)?
    } else {
        DevelopDocument::new(source_ref(&decoded))
    };
    let rgb = develop(&decoded.mosaic, &decoded.profile, &doc)?;
    let dest = out.unwrap_or_else(|| path.with_extension("pfm"));
    write_pfm(&dest, &rgb)?;
    println!(
        "developed {}×{} {:?} → {}",
        rgb.width,
        rgb.height,
        rgb.space,
        dest.display()
    );
    Ok(())
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| epikos_core::Error::Decode(e.to_string()))?;
    println!("{text}");
    Ok(())
}

fn export_cmd(path: &Path, out: &Path, ev: Option<f32>, options: ExportOptions) -> Result<()> {
    let engine = Engine::new(1);
    let mut adjustments = engine.open(path)?.document.adjustments;
    if let Some(ev) = ev {
        adjustments.exposure = ev;
    }
    let r = engine.export(path, &adjustments, out, options)?;
    println!(
        "exported {} {}×{} {} → {} ({:.1} MB; develop {} ms, write {} ms)",
        r.format.label(),
        r.width,
        r.height,
        r.color_space,
        r.path,
        r.bytes as f64 / 1e6,
        r.develop_ms,
        r.write_ms
    );
    println!(
        "metadata    EXIF {}, GPS {}",
        if r.wrote_exif { "written" } else { "none" },
        if r.wrote_location {
            "written"
        } else {
            "not written"
        }
    );
    if !r.alpha_channels.is_empty() {
        println!("channels    {}", r.alpha_channels.join(", "));
    }
    Ok(())
}

fn write_pfm(path: &Path, img: &ImageRgbF32) -> Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    writeln!(f, "PF")?;
    writeln!(f, "{} {}", img.width, img.height)?;
    writeln!(f, "-1.0")?;
    for y in (0..img.height).rev() {
        for x in 0..img.width {
            let p = img.get(x, y);
            f.write_all(&p.r.to_le_bytes())?;
            f.write_all(&p.g.to_le_bytes())?;
            f.write_all(&p.b.to_le_bytes())?;
        }
    }
    Ok(())
}
