use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use epikos_core::{ImageRgbF32, Result};
use epikos_decode::decode_file;
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
    }
}

fn inspect(path: &Path) -> Result<()> {
    let decoded = decode_file(path)?;
    let p = &decoded.profile;
    println!("file        {}", decoded.source_path);
    println!("sha256      {}", decoded.source_sha256);
    println!("format      {}", p.format.label());
    println!("camera      {} {}", p.clean_make, p.clean_model);
    println!("sensor      {}×{}  {} bps  {} spp", p.width, p.height, p.bits_per_sample, p.samples_per_pixel);
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
