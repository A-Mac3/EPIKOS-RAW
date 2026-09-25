//! Step 1 lens profile database: the Lensfun calibrations (CC BY-SA 3.0, see
//! `data/lensfun/LICENSE.md`) compiled into the binary and parsed on first use.
//!
//! A capture's lens is found from its EXIF lens model (token match, with the focal
//! range as a hard constraint), and its corrections are interpolated to the capture's
//! focal length (and, for vignetting, aperture). Fisheye and other non-rectilinear
//! lenses are skipped: correcting them needs a projection change, not a radial warp.

use std::sync::OnceLock;

use epikos_core::{CaptureMetadata, LensGeometry, LensProfile, LensVignetting, Radial};

macro_rules! db_files {
    ($($name:literal),* $(,)?) => {
        &[$(include_str!(concat!("../data/lensfun/", $name))),*]
    };
}

const FILES: &[&str] = db_files![
    "6x6.xml", "actioncams.xml", "compact-canon.xml", "compact-casio.xml", "compact-fujifilm.xml",
    "compact-kodak.xml", "compact-konica-minolta.xml", "compact-leica.xml", "compact-nikon.xml",
    "compact-olympus.xml", "compact-panasonic.xml", "compact-pentax.xml", "compact-ricoh.xml",
    "compact-samsung.xml", "compact-sigma.xml", "compact-sony.xml", "contax.xml", "generic.xml",
    "mil-canon.xml", "mil-fujifilm.xml", "mil-hasselblad.xml", "mil-leica.xml", "mil-nikon.xml",
    "mil-olympus.xml", "mil-panasonic.xml", "mil-pentax.xml", "mil-samsung.xml", "mil-samyang.xml",
    "mil-sigma.xml", "mil-sony.xml", "mil-tamron.xml", "mil-tokina.xml", "mil-zeiss.xml", "misc.xml",
    "om-system.xml", "rf-leica.xml", "slr-canon.xml", "slr-hasselblad.xml", "slr-konica-minolta.xml",
    "slr-leica.xml", "slr-nikon.xml", "slr-olympus.xml", "slr-panasonic.xml", "slr-pentax.xml",
    "slr-ricoh.xml", "slr-samsung.xml", "slr-samyang.xml", "slr-schneider.xml", "slr-sigma.xml",
    "slr-soligor.xml", "slr-sony.xml", "slr-tamron.xml", "slr-tokina.xml", "slr-ussr.xml",
    "slr-vivitar.xml", "slr-zeiss.xml",
];

#[derive(Debug)]
struct Lens {
    maker: String,
    models: Vec<String>,
    crop: f64,
    aspect: f64,
    distortion: Vec<(f64, Radial)>,
    tca: Vec<(f64, [Radial; 2])>,
    /// (focal, aperture, distance, k1, k2, k3)
    vignetting: Vec<[f64; 6]>,
}

#[derive(Debug)]
struct Camera {
    maker: String,
    model: String,
    crop: f64,
}

struct Db {
    lenses: Vec<Lens>,
    cameras: Vec<Camera>,
}

fn db() -> &'static Db {
    static DB: OnceLock<Db> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = Db { lenses: Vec::new(), cameras: Vec::new() };
        for xml in FILES {
            parse_file(xml, &mut db);
        }
        db
    })
}

/// Number of lenses in the database (for the UI).
pub fn lens_count() -> usize {
    db().lenses.len()
}

/// The Lensfun profile for this capture, if its lens is in the database.
pub fn lookup(meta: &CaptureMetadata) -> Option<LensProfile> {
    let model = meta.lens_model.as_deref()?.trim();
    let focal = meta.focal_length.and_then(ratio)?;
    let lens = find_lens(model, meta.lens_make.as_deref().unwrap_or(&meta.make), focal)?;
    let image_crop = find_camera(&meta.make, &meta.model).map_or(lens.crop, |c| c.crop);
    let aperture = meta.f_number.and_then(ratio);

    let distortion = interpolate(&lens.distortion, focal, lerp_radial);
    let tca = interpolate(&lens.tca, focal, |a, b, t| [lerp_radial(a[0], b[0], t), lerp_radial(a[1], b[1], t)]);
    let geometry = (distortion.is_some() || tca.is_some()).then(|| LensGeometry::Hugin {
        scale: lens.crop / image_crop,
        calib_aspect: lens.aspect,
        distortion,
        tca,
    });
    let vignetting = vignetting_at(&lens.vignetting, focal, aperture);
    let profile = LensProfile {
        source: format!("Lensfun: {} {}", lens.maker, lens.models[0]),
        geometry,
        vignetting,
    };
    (!profile.is_empty()).then_some(profile)
}

fn ratio((n, d): (u32, u32)) -> Option<f64> {
    (d != 0 && n != 0).then(|| n as f64 / d as f64)
}

// ---- Matching ------------------------------------------------------------------------

/// Lowercase alphanumeric tokens, split at letter/digit boundaries; decimals stay whole
/// ("f/2.8" → ["f", "2.8"]).
fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_digit = None;
    let chars: Vec<char> = s.to_lowercase().chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let decimal_point = c == '.'
            && prev_digit == Some(true)
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if c.is_alphanumeric() || decimal_point {
            let digit = c.is_ascii_digit() || decimal_point;
            if prev_digit.is_some_and(|p| p != digit) && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(c);
            prev_digit = Some(digit);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_digit = None;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    // "mmF2.8": the unit runs straight into the aperture.
    out.into_iter()
        .flat_map(|t| match t.strip_prefix("mm") {
            Some(rest) if !rest.is_empty() => vec!["mm".to_string(), rest.to_string()],
            _ => vec![t],
        })
        .collect()
}

/// "16-55mm" / "50mm" → (16, 55) / (50, 50).
fn focal_range(s: &str) -> Option<(f64, f64)> {
    let t = tokens(s);
    let mm = t.iter().position(|x| x == "mm")?;
    let hi: f64 = t.get(mm.checked_sub(1)?)?.parse().ok()?;
    let lo = mm
        .checked_sub(2)
        .and_then(|i| t.get(i))
        .and_then(|x| x.parse::<f64>().ok())
        .filter(|lo| *lo < hi && s.contains('-'))
        .unwrap_or(hi);
    Some((lo, hi))
}

fn find_lens<'a>(model: &str, maker: &str, focal: f64) -> Option<&'a Lens> {
    let want = tokens(model);
    let want_range = focal_range(model);
    let maker_l = maker.to_lowercase();
    let mut best: Option<(f64, &Lens)> = None;
    for lens in &db().lenses {
        let same_maker = !maker_l.is_empty()
            && (maker_l.contains(&lens.maker.to_lowercase()) || lens.maker.to_lowercase().contains(&maker_l));
        for name in &lens.models {
            let range = focal_range(name);
            if let (Some(a), Some(b)) = (want_range, range) {
                if (a.0 - b.0).abs() > 0.5 || (a.1 - b.1).abs() > 0.5 {
                    continue;
                }
            }
            if let Some((lo, hi)) = range {
                if focal < lo - 1.0 || focal > hi + 1.0 {
                    continue;
                }
            }
            let have = tokens(name);
            let common = want.iter().filter(|t| have.contains(t)).count() as f64;
            let union = (want.len() + have.len()) as f64 - common;
            let mut score = common / union.max(1.0);
            if same_maker {
                score += 0.15;
            }
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, lens));
            }
        }
    }
    // Below this the names share little more than the focal range.
    best.filter(|(s, _)| *s >= 0.55).map(|(_, l)| l)
}

fn find_camera<'a>(make: &str, model: &str) -> Option<&'a Camera> {
    let (make, model) = (make.to_lowercase(), tokens(model));
    db().cameras.iter().find(|c| {
        let maker = c.maker.to_lowercase();
        (make.contains(&maker) || maker.contains(&make)) && tokens(&c.model) == model
    })
}

// ---- Interpolation -------------------------------------------------------------------

fn interpolate<T: Copy>(calib: &[(f64, T)], focal: f64, lerp: impl Fn(T, T, f64) -> T) -> Option<T> {
    let below = calib.iter().filter(|(f, _)| *f <= focal).max_by(|a, b| a.0.total_cmp(&b.0));
    let above = calib.iter().filter(|(f, _)| *f >= focal).min_by(|a, b| a.0.total_cmp(&b.0));
    match (below, above) {
        (Some(a), Some(b)) if b.0 > a.0 => Some(lerp(a.1, b.1, (focal - a.0) / (b.0 - a.0))),
        (Some(a), _) | (None, Some(a)) => Some(a.1),
        (None, None) => None,
    }
}

/// Coefficients interpolate linearly when both ends use the same model; otherwise the
/// nearer calibration wins.
fn lerp_radial(a: Radial, b: Radial, t: f64) -> Radial {
    let l = |x: f64, y: f64| x + (y - x) * t;
    match (a, b) {
        (Radial::Poly3 { k1: a }, Radial::Poly3 { k1: b }) => Radial::Poly3 { k1: l(a, b) },
        (Radial::Poly5 { k1: a1, k2: a2 }, Radial::Poly5 { k1: b1, k2: b2 }) => {
            Radial::Poly5 { k1: l(a1, b1), k2: l(a2, b2) }
        }
        (Radial::PtLens { a: a1, b: b1, c: c1 }, Radial::PtLens { a: a2, b: b2, c: c2 }) => {
            Radial::PtLens { a: l(a1, a2), b: l(b1, b2), c: l(c1, c2) }
        }
        (Radial::Linear { k: a }, Radial::Linear { k: b }) => Radial::Linear { k: l(a, b) },
        (Radial::TcaPoly3 { b: b1, c: c1, v: v1 }, Radial::TcaPoly3 { b: b2, c: c2, v: v2 }) => {
            Radial::TcaPoly3 { b: l(b1, b2), c: l(c1, c2), v: l(v1, v2) }
        }
        _ if t < 0.5 => a,
        _ => b,
    }
}

/// Nearest calibrated focal length, then the nearest aperture (in stops), at the
/// farthest calibrated distance.
fn vignetting_at(calib: &[[f64; 6]], focal: f64, aperture: Option<f64>) -> Option<LensVignetting> {
    let nearest_focal = calib.iter().map(|v| v[0]).min_by(|a, b| (a - focal).abs().total_cmp(&(b - focal).abs()))?;
    let at_focal: Vec<&[f64; 6]> = calib.iter().filter(|v| v[0] == nearest_focal).collect();
    let stops = |n: f64| 2.0 * n.max(0.1).log2();
    let v = match aperture {
        Some(n) => at_focal.iter().min_by(|a, b| {
            (stops(a[1]) - stops(n)).abs().total_cmp(&(stops(b[1]) - stops(n)).abs()).then(b[2].total_cmp(&a[2]))
        }),
        // Unknown aperture: the most stopped-down calibration (mildest fall-off).
        None => at_focal.iter().max_by(|a, b| a[1].total_cmp(&b[1]).then(a[2].total_cmp(&b[2]))),
    }?;
    Some(LensVignetting::Pa { k1: v[3], k2: v[4], k3: v[5] })
}

// ---- XML -----------------------------------------------------------------------------

fn parse_file(xml: &str, db: &mut Db) {
    let xml = strip_comments(xml);
    for block in blocks(&xml, "lens") {
        if let Some(lens) = parse_lens(block) {
            db.lenses.push(lens);
        }
    }
    for block in blocks(&xml, "camera") {
        let (Some(maker), Some(model), Some(crop)) = (
            element(block, "maker").next(),
            element(block, "model").next(),
            element(block, "cropfactor").next().and_then(|v| v.trim().parse().ok()),
        ) else {
            continue;
        };
        db.cameras.push(Camera { maker: maker.trim().into(), model: model.trim().into(), crop });
    }
}

fn parse_lens(block: &str) -> Option<Lens> {
    let kind = element(block, "type").next().map(|t| t.trim().to_lowercase());
    if kind.is_some_and(|t| t != "rectilinear") {
        return None;
    }
    let maker = element(block, "maker").next()?.trim().to_string();
    // The untranslated model name first: that's the one cameras write to EXIF.
    let models: Vec<String> = element(block, "model").map(|m| m.trim().to_string()).collect();
    if models.is_empty() {
        return None;
    }
    let crop = element(block, "cropfactor").next().and_then(|v| v.trim().parse().ok()).unwrap_or(1.0);
    let aspect = element(block, "aspect-ratio")
        .next()
        .and_then(|v| {
            let (a, b) = v.trim().split_once(':')?;
            Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?)
        })
        .unwrap_or(1.5);
    let mut lens = Lens { maker, models, crop, aspect, distortion: Vec::new(), tca: Vec::new(), vignetting: Vec::new() };
    for tag in empty_tags(block, "distortion") {
        let a = |k: &str| attr(tag, k);
        let Some(focal) = a("focal") else { continue };
        let model = match tag_attr(tag, "model") {
            Some("poly3") => Radial::Poly3 { k1: a("k1").unwrap_or(0.0) },
            Some("poly5") => Radial::Poly5 { k1: a("k1").unwrap_or(0.0), k2: a("k2").unwrap_or(0.0) },
            Some("ptlens") => Radial::PtLens { a: a("a").unwrap_or(0.0), b: a("b").unwrap_or(0.0), c: a("c").unwrap_or(0.0) },
            _ => continue,
        };
        lens.distortion.push((focal, model));
    }
    for tag in empty_tags(block, "tca") {
        let a = |k: &str| attr(tag, k);
        let Some(focal) = a("focal") else { continue };
        let pair = match tag_attr(tag, "model") {
            Some("linear") => [Radial::Linear { k: a("kr").unwrap_or(1.0) }, Radial::Linear { k: a("kb").unwrap_or(1.0) }],
            Some("poly3") => [
                Radial::TcaPoly3 { b: a("br").unwrap_or(0.0), c: a("cr").unwrap_or(0.0), v: a("vr").unwrap_or(1.0) },
                Radial::TcaPoly3 { b: a("bb").unwrap_or(0.0), c: a("cb").unwrap_or(0.0), v: a("vb").unwrap_or(1.0) },
            ],
            _ => continue,
        };
        lens.tca.push((focal, pair));
    }
    for tag in empty_tags(block, "vignetting") {
        let a = |k: &str| attr(tag, k);
        if tag_attr(tag, "model") != Some("pa") {
            continue;
        }
        let (Some(f), Some(n)) = (a("focal"), a("aperture")) else { continue };
        lens.vignetting.push([
            f,
            n,
            a("distance").unwrap_or(1000.0),
            a("k1").unwrap_or(0.0),
            a("k2").unwrap_or(0.0),
            a("k3").unwrap_or(0.0),
        ]);
    }
    (!lens.distortion.is_empty() || !lens.tca.is_empty() || !lens.vignetting.is_empty()).then_some(lens)
}

fn strip_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        rest = match rest[i..].find("-->") {
            Some(j) => &rest[i + j + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Contents of every `<name>…</name>` block (not nested).
fn blocks<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let body = &rest[i + open.len()..];
        let Some(j) = body.find(&close) else { break };
        out.push(&body[..j]);
        rest = &body[j + close.len()..];
    }
    out
}

/// Text of each `<name …>text</name>` element.
fn element<'a>(xml: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut rest = xml;
    std::iter::from_fn(move || loop {
        let i = rest.find(&open)?;
        let after = &rest[i + open.len()..];
        // `<model>` must not match `<models>`.
        if !after.starts_with('>') && !after.starts_with(' ') {
            rest = after;
            continue;
        }
        let gt = after.find('>')?;
        let body = &after[gt + 1..];
        let end = body.find(&close)?;
        rest = &body[end + close.len()..];
        return Some(&body[..end]);
    })
}

/// Attribute text of each `<name …/>` tag.
fn empty_tags<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name} ");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let body = &rest[i + open.len()..];
        let Some(end) = body.find('>') else { break };
        out.push(&body[..end]);
        rest = &body[end..];
    }
    out
}

fn tag_attr<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key}=\"");
    let mut from = 0;
    loop {
        let i = from + tag[from..].find(&needle)?;
        // Whole attribute names only (`b=` must not match `vb=`).
        if i == 0 || tag.as_bytes()[i - 1].is_ascii_whitespace() {
            let v = &tag[i + needle.len()..];
            return Some(&v[..v.find('"')?]);
        }
        from = i + needle.len();
    }
}

fn attr(tag: &str, key: &str) -> Option<f64> {
    tag_attr(tag, key)?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(make: &str, model: &str, lens: &str, focal: f64, f: f64) -> CaptureMetadata {
        CaptureMetadata {
            make: make.into(),
            model: model.into(),
            lens_model: Some(lens.into()),
            focal_length: Some(((focal * 10.0) as u32, 10)),
            f_number: Some(((f * 10.0) as u32, 10)),
            ..Default::default()
        }
    }

    #[test]
    fn database_loads() {
        assert!(lens_count() > 1000, "{}", lens_count());
        assert!(db().cameras.len() > 800);
    }

    #[test]
    fn tokens_split_at_letter_digit_boundaries() {
        assert_eq!(tokens("XF16-55mmF2.8 R"), ["xf", "16", "55", "mm", "f", "2.8", "r"]);
        assert_eq!(focal_range("XF 50-140mm f/2.8"), Some((50.0, 140.0)));
        assert_eq!(focal_range("EF 50mm f/1.8"), Some((50.0, 50.0)));
    }

    #[test]
    fn finds_the_fuji_zoom_and_interpolates_to_the_capture_focal() {
        let m = meta("FUJIFILM", "X-H2", "XF50-140mmF2.8 R LM OIS WR", 140.0, 4.0);
        let p = lookup(&m).expect("in the database");
        assert!(p.source.contains("XF50-140"), "{}", p.source);
        let Some(LensGeometry::Hugin { distortion: Some(Radial::PtLens { a, b, c }), scale, .. }) = p.geometry else {
            panic!("{:?}", p.geometry)
        };
        // Exactly the 140 mm calibration; the X-H2 and the lens share a crop factor.
        assert!((a - 0.00299).abs() < 1e-9 && (b - 0.00907).abs() < 1e-9 && (c + 0.0176).abs() < 1e-9);
        assert!((scale - 1.529 / 1.528).abs() < 1e-6);
        // Halfway between the 50 and 62 mm calibrations.
        let p = lookup(&meta("FUJIFILM", "X-H2", "XF50-140mmF2.8 R LM OIS WR", 56.0, 4.0)).unwrap();
        let Some(LensGeometry::Hugin { distortion: Some(Radial::PtLens { a, .. }), .. }) = p.geometry else { panic!() };
        assert!((a - (0.01685 + -0.00912) / 2.0).abs() < 1e-6, "{a}");
    }

    #[test]
    fn unknown_lenses_and_wrong_focal_ranges_find_nothing() {
        assert!(lookup(&meta("Acme", "Cam", "Mystery 33mm f/1.1", 33.0, 1.1)).is_none());
        // Right name family, but a focal length outside the lens's range.
        assert!(lookup(&meta("FUJIFILM", "X-H2", "XF50-140mmF2.8 R LM OIS WR", 20.0, 4.0)).is_none());
    }

    #[test]
    fn attribute_names_match_whole() {
        assert_eq!(tag_attr(r#"model="poly3" vb="0.99" bb="0.1""#, "b"), None);
        assert_eq!(tag_attr(r#"model="poly3" vb="0.99" b="0.1""#, "b"), Some("0.1"));
    }
}
