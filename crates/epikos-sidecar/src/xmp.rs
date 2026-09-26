use std::fs;
use std::path::{Path, PathBuf};

use epikos_core::{Error, Result};

use crate::document::{
    Adjustments, Atmosphere, BackgroundTint, ChromaticAberration, ColorGrade, ColorWheel, ColorWheels, Crop, Curves,
    DemosaicMode, DevelopDocument, DistortionCoeffs, Finishing, HslBands, HslChannel, LensCorrections,
    LocalAdjustment, LutRef, ManualAdjustment, MaskTarget, NoiseReduction, SCurve, SourceRef, SplitToning, StyleRef, StyleWeight, Texture, Tone,
    ToneCurve, VirtualLight, WbMode, WhiteBalance,
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
    epikos:caBlue="{ca_b}"
    epikos:lensProfile="{lens_profile}"
    epikos:geometry="{rotation},{vertical}"
    epikos:crop="{crop_x},{crop_y},{crop_w},{crop_h}"
    epikos:cropAspect="{crop_aspect}"
    epikos:tone="{tone}"
    epikos:dehaze="{dehaze}"{look}/>
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
        lens_profile = a.lens.profile,
        rotation = a.lens.rotation,
        vertical = a.lens.vertical,
        crop_x = a.crop.x,
        crop_y = a.crop.y,
        crop_w = a.crop.width,
        crop_h = a.crop.height,
        crop_aspect = esc(&a.crop.aspect),
        tone = a.tone.to_array().map(|v| v.to_string()).join(","),
        dehaze = a.tone.dehaze,
        look = render_look(a),
    )
}

/// Steps 4–5 and the style. Camera Raw's Clarity, Texture, HSL and Color Grading use
/// other algorithms and hue bands, so none of these map to `crs:` fields.
fn render_look(a: &Adjustments) -> String {
    let t = &a.texture;
    let mut out = format!(
        "\n    epikos:clarity=\"{}\"\n    epikos:microTexture=\"{}\"\n    epikos:blemishSmoothing=\"{}\"\n    epikos:specularBalance=\"{}\"\n    epikos:characterLines=\"{}\"\n    epikos:retouchSubjectOnly=\"{}\"",
        t.clarity, t.micro_texture, t.blemish_smoothing, t.specular_balance, t.character_lines, t.retouch_subject_only
    );
    // Hand-drawn masks: their shapes don't fit a flat list, so they're kept as JSON.
    if !a.manual.is_empty() {
        if let Ok(json) = serde_json::to_string(&a.manual) {
            out += &format!("\n    epikos:manual=\"{}\"", esc(&json));
        }
    }
    // Local adjustments: "mask,exposure,contrast,saturation,warmth,clarity,tint;…".
    if !a.local.is_empty() {
        let local: Vec<String> = a
            .local
            .iter()
            .map(|l| {
                format!(
                    "{},{},{},{},{},{},{}",
                    l.mask.id(),
                    l.exposure,
                    l.contrast,
                    l.saturation,
                    l.warmth,
                    l.clarity,
                    l.tint
                )
            })
            .collect();
        out += &format!("\n    epikos:local=\"{}\"", local.join(";"));
    }
    let (f, bg) = (&a.color.foliage, &a.color.background);
    out += &format!(
        "\n    epikos:foliage=\"{},{},{}\"\n    epikos:background=\"{},{},{},{}\"",
        f.hue, f.saturation, f.luminance, bg.hue, bg.amount, bg.saturation, bg.luminance
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
    for (name, wheel) in [
        ("Shadows", &w.shadows),
        ("Midtones", &w.midtones),
        ("Highlights", &w.highlights),
    ] {
        out += &format!(
            "\n    epikos:wheel{name}=\"{},{},{}\"",
            wheel.hue, wheel.amount, wheel.luminance
        );
    }
    let at = &a.atmosphere;
    out += &format!(
        "\n    epikos:glow=\"{},{},{}\"\n    epikos:fog=\"{},{},{}\"\n    epikos:shafts=\"{},{},{}\"\n    epikos:shaftLight=\"{},{},{}\"",
        at.glow, at.glow_size, at.glow_warmth,
        at.fog, at.fog_start, at.fog_warmth,
        at.shafts, at.shaft_length, at.shaft_warmth,
        if at.shaft_auto { "auto" } else { "manual" }, at.shaft_x, at.shaft_y
    );
    if at.glow_subject_only {
        out += "\n    epikos:glowSubjectOnly=\"true\"";
    }
    // Virtual lights: "x,y,depth,intensity,reach,warmth,halo;…".
    if !at.lights.is_empty() {
        let lights: Vec<String> = at
            .lights
            .iter()
            .map(|l| format!("{},{},{},{},{},{},{}", l.x, l.y, l.depth, l.intensity, l.reach, l.warmth, l.halo))
            .collect();
        out += &format!("\n    epikos:lights=\"{}\"", lights.join(";"));
    }
    let c = &a.curves;
    for (name, curve) in [
        ("Rgb", &c.rgb),
        ("Red", &c.red),
        ("Green", &c.green),
        ("Blue", &c.blue),
    ] {
        let v = curve.to_array().map(|x| x.to_string()).join(",");
        out += &format!("\n    epikos:curve{name}=\"{v}\"");
        if !curve.points.is_empty() {
            let pts: Vec<String> = curve.points.iter().map(|[x, y]| format!("{x} {y}")).collect();
            out += &format!("\n    epikos:curvePoints{name}=\"{}\"", pts.join(";"));
        }
    }
    let sc = &c.s_curve;
    if *sc != SCurve::default() {
        out += &format!("\n    epikos:sCurve=\"{},{},{}\"", sc.enabled, sc.amount, sc.pivot);
    }
    let f = &a.finishing;
    out += &format!(
        "\n    epikos:grain=\"{},{},{}\"\n    epikos:vignette=\"{},{},{},{}\"",
        f.grain,
        f.grain_size,
        f.grain_roughness,
        f.vignette,
        f.vignette_midpoint,
        f.vignette_feather,
        f.vignette_roundness
    );
    let st = &a.split_toning;
    out += &format!(
        "\n    epikos:splitToning=\"{},{},{},{},{}\"",
        st.highlight_hue, st.highlight_saturation, st.shadow_hue, st.shadow_saturation, st.balance
    );
    out += &format!(
        "\n    epikos:skinProtection=\"{}\"\n    epikos:style=\"{}\"\n    epikos:styleAmount=\"{}\"\n    epikos:styleSkinProtection=\"{}\"",
        a.color.skin_protection,
        esc(&a.style.id),
        a.style.amount,
        a.style.skin_protection
    );
    if !a.lut.name.is_empty() {
        out += &format!("\n    epikos:lut=\"{}\"\n    epikos:lutAmount=\"{}\"", esc(&a.lut.name), a.lut.amount);
    }
    if !a.style.blend.is_empty() {
        let blend: Vec<String> = a.style.blend.iter().map(|b| format!("{}:{}", esc(&b.id), b.weight)).collect();
        out += &format!("\n    epikos:styleBlend=\"{}\"", blend.join(";"));
    }
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
    numbers(s)
}

/// Exactly `N` comma-separated numbers.
fn numbers<const N: usize>(s: &str) -> Option<[f32; N]> {
    let v: Vec<f32> = s
        .split(',')
        .map(|v| v.trim().parse::<f32>().ok())
        .collect::<Option<_>>()?;
    v.try_into().ok()
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
            tone: Tone {
                dehaze: num("epikos:dehaze").unwrap_or(0.0),
                ..get("epikos:tone").and_then(numbers::<7>).map(Tone::from_array).unwrap_or_default()
            },
            crop: get("epikos:crop")
                .and_then(numbers::<4>)
                .map(|[x, y, width, height]| Crop {
                    x,
                    y,
                    width,
                    height,
                    aspect: get("epikos:cropAspect").map(unescape).unwrap_or_else(|| "free".into()),
                })
                .unwrap_or_default(),
            local: get("epikos:local")
                .map(|v| {
                    v.split(';')
                        .filter_map(|l| {
                            let (mask, rest) = l.split_once(',')?;
                            // Tint came later: five values in older sidecars.
                            let [exposure, contrast, saturation, warmth, clarity, tint] = numbers::<6>(rest)
                                .or_else(|| numbers::<5>(rest).map(|[e, c, s, w, k]| [e, c, s, w, k, 0.0]))?;
                            Some(LocalAdjustment {
                                mask: MaskTarget::from_id(mask.trim())?,
                                exposure,
                                contrast,
                                saturation,
                                warmth,
                                clarity,
                                tint,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            // Camera Raw's LuminanceSmoothing / ColorNoiseReduction use a different
            // algorithm, so the numbers don't transfer; only EPIKOS values are read.
            noise_reduction: NoiseReduction {
                luminance: num("epikos:nrLuminance").unwrap_or(defaults.noise_reduction.luminance),
                color: num("epikos:nrColor").unwrap_or(defaults.noise_reduction.color),
            },
            demosaic,
            texture: Texture {
                clarity: num("epikos:clarity").unwrap_or(0.0),
                micro_texture: num("epikos:microTexture").unwrap_or(0.0),
                blemish_smoothing: num("epikos:blemishSmoothing").unwrap_or(0.0),
                specular_balance: num("epikos:specularBalance").unwrap_or(0.0),
                character_lines: num("epikos:characterLines").unwrap_or(0.0),
                retouch_subject_only: flag("epikos:retouchSubjectOnly").unwrap_or(false),
            },
            color: {
                let mut hsl = HslBands::default();
                for (name, band) in HslBands::NAMES.iter().zip(hsl.bands_mut()) {
                    if let Some([hue, saturation, luminance]) =
                        get(&format!("epikos:hsl{}", capitalise(name))).and_then(triple)
                    {
                        *band = HslChannel {
                            hue,
                            saturation,
                            luminance,
                        };
                    }
                }
                let wheel = |name: &str| {
                    get(&format!("epikos:wheel{name}"))
                        .and_then(triple)
                        .map(|[hue, amount, luminance]| ColorWheel {
                            hue,
                            amount,
                            luminance,
                        })
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
                    foliage: get("epikos:foliage")
                        .and_then(triple)
                        .map(|[hue, saturation, luminance]| HslChannel { hue, saturation, luminance })
                        .unwrap_or_default(),
                    background: get("epikos:background")
                        .and_then(numbers::<4>)
                        .map(|[hue, amount, saturation, luminance]| BackgroundTint { hue, amount, saturation, luminance })
                        .unwrap_or_default(),
                }
            },
            atmosphere: {
                let d = Atmosphere::default();
                let [glow, glow_size, glow_warmth] = get("epikos:glow")
                    .and_then(triple)
                    .unwrap_or([d.glow, d.glow_size, d.glow_warmth]);
                let [fog, fog_start, fog_warmth] = get("epikos:fog").and_then(triple).unwrap_or([
                    d.fog,
                    d.fog_start,
                    d.fog_warmth,
                ]);
                let [shafts, shaft_length, shaft_warmth] = get("epikos:shafts")
                    .and_then(triple)
                    .unwrap_or([d.shafts, d.shaft_length, d.shaft_warmth]);
                let light = get("epikos:shaftLight").and_then(|v| {
                    let (mode, xy) = v.split_once(',')?;
                    let (x, y) = xy.split_once(',')?;
                    Some((
                        mode == "auto",
                        x.trim().parse().ok()?,
                        y.trim().parse().ok()?,
                    ))
                });
                let (shaft_auto, shaft_x, shaft_y) =
                    light.unwrap_or((d.shaft_auto, d.shaft_x, d.shaft_y));
                let lights = get("epikos:lights")
                    .map(|v| {
                        v.split(';')
                            .filter_map(numbers::<7>)
                            .map(|[x, y, depth, intensity, reach, warmth, halo]| VirtualLight {
                                x,
                                y,
                                depth,
                                intensity,
                                reach,
                                warmth,
                                halo,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Atmosphere {
                    lights,
                    glow_subject_only: flag("epikos:glowSubjectOnly").unwrap_or(false),
                    glow,
                    glow_size,
                    glow_warmth,
                    fog,
                    fog_start,
                    fog_warmth,
                    shafts,
                    shaft_length,
                    shaft_warmth,
                    shaft_auto,
                    shaft_x,
                    shaft_y,
                }
            },
            curves: {
                let curve = |name: &str| {
                    let mut c = get(&format!("epikos:curve{name}"))
                        .and_then(numbers::<6>)
                        .map(ToneCurve::from_array)
                        .unwrap_or_default();
                    // "x y;x y;…"
                    c.points = get(&format!("epikos:curvePoints{name}"))
                        .map(|v| {
                            v.split(';')
                                .filter_map(|p| {
                                    let (x, y) = p.trim().split_once(' ')?;
                                    Some([x.parse().ok()?, y.parse().ok()?])
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    c
                };
                Curves {
                    rgb: curve("Rgb"),
                    red: curve("Red"),
                    green: curve("Green"),
                    blue: curve("Blue"),
                    s_curve: get("epikos:sCurve")
                        .and_then(|v| {
                            let mut it = v.split(',');
                            Some(SCurve {
                                enabled: parse_bool(it.next()?.trim())?,
                                amount: it.next()?.trim().parse().ok()?,
                                pivot: it.next()?.trim().parse().ok()?,
                            })
                        })
                        .unwrap_or_default(),
                }
            },
            split_toning: get("epikos:splitToning")
                .and_then(numbers::<5>)
                .map(|[hh, hs, sh, ss, balance]| SplitToning {
                    highlight_hue: hh,
                    highlight_saturation: hs,
                    shadow_hue: sh,
                    shadow_saturation: ss,
                    balance,
                })
                .unwrap_or_default(),
            finishing: {
                let d = Finishing::default();
                let [grain, grain_size, grain_roughness] = get("epikos:grain")
                    .and_then(triple)
                    .unwrap_or([d.grain, d.grain_size, d.grain_roughness]);
                let [vignette, vignette_midpoint, vignette_feather, vignette_roundness] =
                    get("epikos:vignette").and_then(numbers::<4>).unwrap_or([
                        d.vignette,
                        d.vignette_midpoint,
                        d.vignette_feather,
                        d.vignette_roundness,
                    ]);
                Finishing {
                    grain,
                    grain_size,
                    grain_roughness,
                    vignette,
                    vignette_midpoint,
                    vignette_feather,
                    vignette_roundness,
                }
            },
            style: StyleRef {
                id: text("epikos:style"),
                amount: num("epikos:styleAmount").unwrap_or(defaults.style.amount),
                skin_protection: num("epikos:styleSkinProtection")
                    .unwrap_or(defaults.style.skin_protection),
                blend: get("epikos:styleBlend")
                    .map(|v| {
                        v.split(';')
                            .filter_map(|b| {
                                let (id, w) = b.rsplit_once(':')?;
                                Some(StyleWeight { id: unescape(id), weight: w.parse().ok()? })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            manual: get("epikos:manual")
                .and_then(|v| serde_json::from_str::<Vec<ManualAdjustment>>(&unescape(v)).ok())
                .unwrap_or_default(),
            lut: LutRef {
                name: text("epikos:lut"),
                amount: num("epikos:lutAmount").unwrap_or(defaults.lut.amount),
            },
            lens: LensCorrections {
                profile: flag("epikos:lensProfile").unwrap_or(defaults.lens.profile),
                rotation: get("epikos:geometry").and_then(numbers::<2>).map_or(0.0, |g| g[0]),
                vertical: get("epikos:geometry").and_then(numbers::<2>).map_or(0.0, |g| g[1]),
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
    use crate::document::{BrushStroke, ManualShape};
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
    fn older_local_adjustments_without_tint_still_load() {
        let mut doc = sample_doc();
        doc.adjustments.local = vec![LocalAdjustment { mask: MaskTarget::Eyes, exposure: 0.4, ..Default::default() }];
        let xml = render_xmp(&doc).replace("eyes,0.4,0,0,0,0,0", "eyes,0.4,0,0,0,0");
        assert!(xml.contains(r#"epikos:local="eyes,0.4,0,0,0,0""#), "{xml}");
        assert_eq!(parse_xmp(&xml).unwrap().adjustments.local, doc.adjustments.local);
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
        doc.adjustments.atmosphere.glow = 35.0;
        doc.adjustments.atmosphere.fog = 42.5;
        doc.adjustments.atmosphere.fog_warmth = -20.0;
        doc.adjustments.atmosphere.shafts = 60.0;
        doc.adjustments.atmosphere.shaft_auto = false;
        doc.adjustments.atmosphere.shaft_x = 0.8;
        doc.adjustments.atmosphere.shaft_y = 0.05;
        doc.adjustments.curves.rgb.darks = -15.0;
        doc.adjustments.curves.rgb.black = 12.5;
        doc.adjustments.curves.blue.highlights = -8.0;
        doc.adjustments.finishing.grain = 35.0;
        doc.adjustments.atmosphere.lights = vec![VirtualLight { x: 0.7, depth: 0.8, ..Default::default() }];
        doc.adjustments.style.blend = vec![
            StyleWeight { id: "silver-charcoal".into(), weight: 0.5 },
            StyleWeight { id: "volumetric-golden-hour".into(), weight: 0.3 },
        ];
        doc.adjustments.finishing.vignette = -22.5;
        doc.adjustments.finishing.vignette_roundness = 40.0;
        doc.adjustments.curves.red.points = vec![[0.0, 0.05], [0.4, 0.35], [1.0, 1.0]];
        doc.adjustments.split_toning.highlight_saturation = 30.0;
        doc.adjustments.split_toning.shadow_hue = 190.0;
        doc.adjustments.split_toning.balance = -20.0;
        doc.adjustments.style.id = "silver-charcoal".into();
        doc.adjustments.style.amount = 70.0;
        doc.adjustments.style.skin_protection = 55.0;
        doc.adjustments.lens.profile = false;
        doc.adjustments.lens.rotation = -1.25;
        doc.adjustments.lens.vertical = 30.0;
        doc.adjustments.tone = Tone::from_array([10.0, -40.0, 35.5, 5.0, -8.0, 20.0, -3.0]);
        doc.adjustments.tone.dehaze = 35.0;
        doc.adjustments.manual = vec![
            ManualAdjustment {
                shape: ManualShape::Linear { x0: 0.5, y0: 0.0, x1: 0.5, y1: 0.45 },
                exposure: -0.6,
                dehaze: 30.0,
                ..Default::default()
            },
            ManualAdjustment {
                shape: ManualShape::Brush {
                    strokes: vec![BrushStroke { points: vec![[0.1, 0.2], [0.3, 0.25]], size: 0.04, erase: true, ..Default::default() }],
                },
                warmth: 12.0,
                ..Default::default()
            },
        ];
        doc.adjustments.crop = Crop { x: 0.1, y: 0.05, width: 0.8, height: 0.64, aspect: "4:5".into() };
        doc.adjustments.local = vec![
            LocalAdjustment { mask: MaskTarget::Eyes, exposure: 0.4, clarity: 25.0, ..Default::default() },
            LocalAdjustment { mask: MaskTarget::Background, saturation: -30.0, warmth: 12.0, ..Default::default() },
            LocalAdjustment { mask: MaskTarget::Skin, warmth: -6.0, tint: 4.5, ..Default::default() },
        ];
        doc.adjustments.texture.character_lines = 40.0;
        doc.adjustments.texture.retouch_subject_only = true;
        doc.adjustments.color.foliage = HslChannel { hue: 35.0, saturation: -10.0, luminance: 5.0 };
        doc.adjustments.color.background = BackgroundTint { hue: 200.0, amount: 30.0, saturation: -20.0, luminance: -10.0 };
        doc.adjustments.atmosphere.glow_subject_only = true;
        doc.adjustments.curves.s_curve = SCurve { enabled: true, amount: 35.0, pivot: 42.0 };
        doc.adjustments.lut = LutRef { name: "Kodak 2383 & \"print\"".into(), amount: 65.0 };
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
        assert_eq!(
            doc.adjustments,
            Adjustments {
                white_balance: doc.adjustments.white_balance.clone(),
                ..Adjustments::default()
            }
        );
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
