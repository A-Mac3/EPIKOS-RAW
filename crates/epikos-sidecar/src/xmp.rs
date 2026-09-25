use std::fs;
use std::path::{Path, PathBuf};

use epikos_core::{Error, Result};

use crate::document::{
    Adjustments, ChromaticAberration, ColorGrade, ColorWheel, ColorWheels, DemosaicMode,
    DevelopDocument, DistortionCoeffs, HslBands, HslChannel, LensCorrections, NoiseReduction,
    SourceRef, StyleRef, Texture, WbMode, WhiteBalance,
};

/// XMP namespace that marks a sidecar as written by EPIKOS RAW.
const EPIKOS_NS: &str = "https://epikos.raw/ns/1.0/";

/// Adobe convention: `IMG_0001.CR3` → `IMG_0001.xmp` (Lightroom, Camera Raw, Bridge).
///
/// DNG files are an exception on Adobe's side: Lightroom reads DNG settings from the
/// file itself and ignores a sibling `.xmp`, so for DNG this sidecar is EPIKOS-only.
pub fn sidecar_xmp_path(raw_path: &Path) -> PathBuf {
    raw_path.with_extension("xmp")
}

/// Write the XMP sidecar. Refuses to overwrite an XMP written by another application
/// (e.g. existing Lightroom edits), which would silently destroy the user's work.
pub fn save_xmp(path: &Path, doc: &DevelopDocument) -> Result<()> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.contains(EPIKOS_NS) {
            return Err(Error::Sidecar(format!(
                "{} was written by another application; not overwriting it",
                path.display()
            )));
        }
    }
    fs::write(path, render_xmp(doc))?;
    Ok(())
}

pub fn load_xmp(path: &Path) -> Result<DevelopDocument> {
    let text = fs::read_to_string(path)?;
    parse_xmp(&text)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Adobe fields (`crs:`) carry only settings whose meaning matches Camera Raw exactly;
/// everything else lives in the `epikos:` namespace, which Adobe apps ignore.
fn render_xmp(doc: &DevelopDocument) -> String {
    let a = &doc.adjustments;
    let demosaic = match a.demosaic {
        DemosaicMode::Auto => "auto",
        DemosaicMode::Malvar => "malvar",
        DemosaicMode::Bilinear => "bilinear",
        DemosaicMode::Xtrans => "xtrans",
    };
    let (wb, wb_mode) = match a.white_balance.mode {
        WbMode::AsShot => ("As Shot", "asShot"),
        WbMode::Custom => ("Custom", "custom"),
    };
    // Camera Raw stores temperature/tint as integers and only uses them for Custom.
    let crs_temp_tint = match a.white_balance.mode {
        WbMode::AsShot => String::new(),
        WbMode::Custom => format!(
            "\n    crs:Temperature=\"{}\"\n    crs:Tint=\"{}\"",
            a.white_balance.temperature.round() as i32,
            a.white_balance.tint.round() as i32,
        ),
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="EPIKOS RAW 0.1">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
    xmlns:epikos="{ns}"
    crs:Version="15.0"
    crs:ProcessVersion="11.0"
    crs:HasSettings="True"
    crs:AlreadyApplied="False"
    crs:WhiteBalance="{wb}"{crs_temp_tint}
    epikos:schemaVersion="{ver}"
    epikos:sourcePath="{path}"
    epikos:sourceSha256="{sha}"
    epikos:format="{fmt}"
    epikos:make="{make}"
    epikos:model="{model}"
    epikos:whiteBalance="{wb_mode}"
    epikos:temperature="{temp}"
    epikos:tint="{tint}"
    epikos:exposure="{exposure}"
    epikos:nrLuminance="{nr_luma}"
    epikos:nrColor="{nr_color}"
    epikos:highlightRecovery="{hr}"
    epikos:demosaic="{demosaic}"
    epikos:distortionEnabled="{dist_on}"
    epikos:k1="{k1}"
    epikos:k2="{k2}"
    epikos:k3="{k3}"
    epikos:p1="{p1}"
    epikos:p2="{p2}"
    epikos:cx="{cx}"
    epikos:cy="{cy}"
    epikos:caEnabled="{ca_on}"
    epikos:caRed="{ca_r}"
    epikos:caBlue="{ca_b}"{look}/>
 </rdf:RDF>
</x:xmpmeta>
"#,
        ns = EPIKOS_NS,
        wb = wb,
        crs_temp_tint = crs_temp_tint,
        wb_mode = wb_mode,
        temp = a.white_balance.temperature,
        tint = a.white_balance.tint,
        ver = doc.version,
        path = esc(&doc.source.path),
        sha = esc(&doc.source.sha256),
        fmt = esc(&doc.source.format),
        make = esc(&doc.source.make),
        model = esc(&doc.source.model),
        exposure = a.exposure,
        nr_luma = a.noise_reduction.luminance,
        nr_color = a.noise_reduction.color,
        hr = a.highlight_recovery,
        demosaic = demosaic,
        dist_on = a.lens.distortion.enabled,
        k1 = a.lens.distortion.k1,
        k2 = a.lens.distortion.k2,
        k3 = a.lens.distortion.k3,
        p1 = a.lens.distortion.p1,
        p2 = a.lens.distortion.p2,
        cx = a.lens.distortion.cx,
        cy = a.lens.distortion.cy,
        ca_on = a.lens.chromatic_aberration.enabled,
        ca_r = a.lens.chromatic_aberration.red,
        ca_b = a.lens.chromatic_aberration.blue,
        look = render_look(a),
    )
}

/// Steps 4–5 and the style. Camera Raw's Clarity, Texture, HSL and Color Grading use
/// other algorithms and hue bands, so none of these map to `crs:` fields.
fn render_look(a: &Adjustments) -> String {
    let t = &a.texture;
    let mut out = format!(
        "\n    epikos:clarity=\"{}\"\n    epikos:microTexture=\"{}\"\n    epikos:blemishSmoothing=\"{}\"\n    epikos:specularBalance=\"{}\"",
        t.clarity, t.micro_texture, t.blemish_smoothing, t.specular_balance
    );
    for (name, band) in HslBands::NAMES.iter().zip(a.color.hsl.bands()) {
        out += &format!(
            "\n    epikos:hsl{}=\"{},{},{}\"",
            capitalise(name),
            band.hue,
            band.saturation,
            band.luminance
        );
    }
    let w = &a.color.wheels;
    for (name, wheel) in [("Shadows", &w.shadows), ("Midtones", &w.midtones), ("Highlights", &w.highlights)] {
        out += &format!(
            "\n    epikos:wheel{name}=\"{},{},{}\"",
            wheel.hue, wheel.amount, wheel.luminance
        );
    }
    out += &format!(
        "\n    epikos:skinProtection=\"{}\"\n    epikos:style=\"{}\"\n    epikos:styleAmount=\"{}\"\n    epikos:styleSkinProtection=\"{}\"",
        a.color.skin_protection,
        esc(&a.style.id),
        a.style.amount,
        a.style.skin_protection
    );
    out
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
        .unwrap_or_default()
}

/// `"a,b,c"` → three numbers.
fn triple(s: &str) -> Option<[f32; 3]> {
    let mut it = s.split(',').map(|v| v.trim().parse::<f32>().ok());
    let v = [it.next()??, it.next()??, it.next()??];
    it.next().is_none().then_some(v)
}

fn attr<'a>(xml: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!(r#"{key}=""#);
    let start = xml.find(&needle)? + needle.len();
    let rest = xml.get(start..)?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Parse an EPIKOS XMP, or import white balance from a Camera Raw XMP. Missing
/// attributes fall back to [`Adjustments::default`].
fn parse_xmp(xml: &str) -> Result<DevelopDocument> {
    let get = |k: &str| attr(xml, k);
    let num = |k: &str| get(k).and_then(|v| v.parse::<f32>().ok());
    let flag = |k: &str| get(k).and_then(parse_bool);
    let text = |k: &str| unescape(get(k).unwrap_or(""));
    let defaults = Adjustments::default();

    let demosaic = match get("epikos:demosaic") {
        Some("malvar") => DemosaicMode::Malvar,
        Some("bilinear") => DemosaicMode::Bilinear,
        Some("xtrans") => DemosaicMode::Xtrans,
        _ => DemosaicMode::Auto,
    };

    // Prefer the exact EPIKOS values; fall back to Camera Raw's integer fields. Any
    // Camera Raw preset other than "As Shot" ("Daylight", "Custom", …) carries an
    // explicit temperature, so it maps to Custom when one is present.
    let temperature = num("epikos:temperature").or_else(|| num("crs:Temperature"));
    let tint = num("epikos:tint").or_else(|| num("crs:Tint"));
    let wb_mode = match (get("epikos:whiteBalance"), get("crs:WhiteBalance")) {
        (Some("custom"), _) => WbMode::Custom,
        (Some(_), _) => WbMode::AsShot,
        (None, Some(v)) if !v.eq_ignore_ascii_case("as shot") && temperature.is_some() => {
            WbMode::Custom
        }
        _ => WbMode::AsShot,
    };

    let d = &defaults.lens.distortion;
    let ca = &defaults.lens.chromatic_aberration;
    Ok(DevelopDocument {
        version: get("epikos:schemaVersion")
            .and_then(|v| v.parse().ok())
            .unwrap_or(DevelopDocument::VERSION),
        source: SourceRef {
            path: text("epikos:sourcePath"),
            sha256: text("epikos:sourceSha256"),
            format: text("epikos:format"),
            make: text("epikos:make"),
            model: text("epikos:model"),
        },
        adjustments: Adjustments {
            white_balance: WhiteBalance {
                mode: wb_mode,
                temperature: temperature.unwrap_or(defaults.white_balance.temperature),
                tint: tint.unwrap_or(defaults.white_balance.tint),
            },
            highlight_recovery: flag("epikos:highlightRecovery")
                .unwrap_or(defaults.highlight_recovery),
            exposure: num("epikos:exposure").unwrap_or(defaults.exposure),
            // Camera Raw's LuminanceSmoothing / ColorNoiseReduction use a different
            // algorithm, so the numbers don't transfer; only EPIKOS values are read.
            noise_reduction: NoiseReduction {
                luminance: num("epikos:nrLuminance")
                    .unwrap_or(defaults.noise_reduction.luminance),
                color: num("epikos:nrColor").unwrap_or(defaults.noise_reduction.color),
            },
            demosaic,
            texture: Texture {
                clarity: num("epikos:clarity").unwrap_or(0.0),
                micro_texture: num("epikos:microTexture").unwrap_or(0.0),
                blemish_smoothing: num("epikos:blemishSmoothing").unwrap_or(0.0),
                specular_balance: num("epikos:specularBalance").unwrap_or(0.0),
            },
            color: {
                let mut hsl = HslBands::default();
                for (name, band) in HslBands::NAMES.iter().zip(hsl.bands_mut()) {
                    if let Some([hue, saturation, luminance]) =
                        get(&format!("epikos:hsl{}", capitalise(name))).and_then(triple)
                    {
                        *band = HslChannel { hue, saturation, luminance };
                    }
                }
                let wheel = |name: &str| {
                    get(&format!("epikos:wheel{name}"))
                        .and_then(triple)
                        .map(|[hue, amount, luminance]| ColorWheel { hue, amount, luminance })
                        .unwrap_or_default()
                };
                ColorGrade {
                    hsl,
                    wheels: ColorWheels {
                        shadows: wheel("Shadows"),
                        midtones: wheel("Midtones"),
                        highlights: wheel("Highlights"),
                    },
                    skin_protection: num("epikos:skinProtection").unwrap_or(0.0),
                }
            },
            style: StyleRef {
                id: text("epikos:style"),
                amount: num("epikos:styleAmount").unwrap_or(defaults.style.amount),
                skin_protection: num("epikos:styleSkinProtection")
                    .unwrap_or(defaults.style.skin_protection),
            },
            lens: LensCorrections {
                distortion: DistortionCoeffs {
                    enabled: flag("epikos:distortionEnabled").unwrap_or(d.enabled),
                    k1: num("epikos:k1").unwrap_or(d.k1),
                    k2: num("epikos:k2").unwrap_or(d.k2),
                    k3: num("epikos:k3").unwrap_or(d.k3),
                    p1: num("epikos:p1").unwrap_or(d.p1),
                    p2: num("epikos:p2").unwrap_or(d.p2),
                    cx: num("epikos:cx").unwrap_or(d.cx),
                    cy: num("epikos:cy").unwrap_or(d.cy),
                },
                chromatic_aberration: ChromaticAberration {
                    enabled: flag("epikos:caEnabled").unwrap_or(ca.enabled),
                    red: num("epikos:caRed").unwrap_or(ca.red),
                    blue: num("epikos:caBlue").unwrap_or(ca.blue),
                },
            },
        },
    })
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{DevelopDocument, SourceRef, WbMode};
    use epikos_core::Error;

    fn sample_doc() -> DevelopDocument {
        DevelopDocument::new(SourceRef {
            path: "clip.DNG".into(),
            sha256: "deadbeef".into(),
            format: "Apple ProRAW".into(),
            make: "Apple".into(),
            model: "iPhone 15 Pro".into(),
        })
    }

    #[test]
    fn xmp_roundtrip_preserves_adjustments() {
        let mut doc = sample_doc();
        doc.adjustments.highlight_recovery = true;
        doc.adjustments.exposure = 0.75;
        doc.adjustments.noise_reduction.luminance = 40.0;
        doc.adjustments.noise_reduction.color = 10.0;
        doc.adjustments.white_balance.mode = WbMode::Custom;
        doc.adjustments.white_balance.temperature = 5200.5;
        doc.adjustments.white_balance.tint = 8.25;
        doc.adjustments.lens.distortion.enabled = true;
        doc.adjustments.lens.distortion.k1 = -0.12;
        doc.adjustments.lens.chromatic_aberration.enabled = true;
        doc.adjustments.lens.chromatic_aberration.red = 0.004;
        doc.adjustments.lens.chromatic_aberration.blue = -0.003;
        doc.adjustments.texture.clarity = -12.5;
        doc.adjustments.texture.micro_texture = 30.0;
        doc.adjustments.texture.blemish_smoothing = 45.0;
        doc.adjustments.texture.specular_balance = 20.0;
        doc.adjustments.color.hsl.orange.hue = -5.0;
        doc.adjustments.color.hsl.blue.saturation = 22.0;
        doc.adjustments.color.hsl.magenta.luminance = -7.25;
        doc.adjustments.color.wheels.highlights.hue = 45.0;
        doc.adjustments.color.wheels.highlights.amount = 18.0;
        doc.adjustments.color.wheels.shadows.luminance = -10.0;
        doc.adjustments.color.skin_protection = 60.0;
        doc.adjustments.style.id = "silver-charcoal".into();
        doc.adjustments.style.amount = 70.0;
        doc.adjustments.style.skin_protection = 55.0;
        let xml = render_xmp(&doc);
        let back = parse_xmp(&xml).unwrap();
        assert_eq!(doc, back);
    }

    #[test]
    fn xmp_uses_camera_raw_values_and_omits_mismatched_fields() {
        let mut doc = sample_doc();
        let xml = render_xmp(&doc);
        assert!(xml.contains(r#"crs:WhiteBalance="As Shot""#));
        assert!(!xml.contains("crs:Temperature"));
        assert!(!xml.contains("crs:LensProfileEnable"));
        assert!(!xml.contains("crs:ChromaticAberration"));

        doc.adjustments.white_balance.mode = WbMode::Custom;
        doc.adjustments.white_balance.temperature = 5200.4;
        doc.adjustments.white_balance.tint = -7.6;
        let xml = render_xmp(&doc);
        assert!(xml.contains(r#"crs:WhiteBalance="Custom""#));
        assert!(xml.contains(r#"crs:Temperature="5200""#));
        assert!(xml.contains(r#"crs:Tint="-8""#));
    }

    #[test]
    fn imports_white_balance_from_a_lightroom_xmp() {
        let xml = r#"<rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
            crs:WhiteBalance="Daylight" crs:Temperature="5500" crs:Tint="10"/>"#;
        let doc = parse_xmp(xml).unwrap();
        assert_eq!(doc.adjustments.white_balance.mode, WbMode::Custom);
        assert_eq!(doc.adjustments.white_balance.temperature, 5500.0);
        assert_eq!(doc.adjustments.white_balance.tint, 10.0);
        assert_eq!(doc.adjustments, Adjustments {
            white_balance: doc.adjustments.white_balance.clone(),
            ..Adjustments::default()
        });
    }

    #[test]
    fn sidecar_path_follows_adobe_naming() {
        assert_eq!(
            sidecar_xmp_path(Path::new("/shoot/IMG_0001.CR3")),
            PathBuf::from("/shoot/IMG_0001.xmp")
        );
    }

    #[test]
    fn refuses_to_overwrite_another_apps_xmp() {
        let dir = std::env::temp_dir().join(format!("epikos-xmp-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("IMG.xmp");
        let foreign = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core"/>"#;
        fs::write(&path, foreign).unwrap();

        let err = save_xmp(&path, &sample_doc()).unwrap_err();
        assert!(matches!(err, Error::Sidecar(_)), "got {err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), foreign);

        // Our own sidecar may be rewritten.
        fs::remove_file(&path).unwrap();
        save_xmp(&path, &sample_doc()).unwrap();
        save_xmp(&path, &sample_doc()).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_xmp_is_io_error() {
        let err = load_xmp(Path::new("/no/such/file.xmp")).unwrap_err();
        match err {
            Error::Io(_) => {}
            other => panic!("expected io, got {other}"),
        }
    }
}
