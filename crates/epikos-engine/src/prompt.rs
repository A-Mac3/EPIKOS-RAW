//! PRD Section 4 "Natural Language Look Prompting": a description such as "an eerie,
//! foggy 1970s Scandinavian film scene with subtle golden light on the face" becomes
//! real slider, curve, wheel and light settings.
//!
//! The interpreter is local and deterministic: a vocabulary of look concepts (moods,
//! weather, light, eras, film stocks, places, colours, skin and texture), each a set of
//! parameter changes. Intensity words scale a concept ("subtle" ½×, "very" 1.6×),
//! negation removes it ("no grain"), and "<colour> light on the face" places a virtual
//! light at the subject. Changes add to the photo's current settings, and every match
//! is reported so the photographer can see exactly what moved.

use epikos_sidecar::{Adjustments, ColorWheel, HslBands, VirtualLight};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LookPrompt {
    pub adjustments: Adjustments,
    /// What was understood, in prompt order.
    pub matched: Vec<PromptMatch>,
    /// Words that weren't recognised (stop words excluded).
    pub unknown: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptMatch {
    /// The words that matched, as typed.
    pub phrase: String,
    /// What it does, e.g. "distance fog".
    pub effect: String,
    /// Applied strength: 0.5 for "subtle", 1.6 for "very", −1 for "no".
    pub strength: f32,
}

type Apply = fn(&mut Adjustments, f32);

struct Concept {
    phrases: &'static [&'static str],
    effect: &'static str,
    apply: Apply,
}

const SOFTER: &[&str] = &["subtle", "slight", "slightly", "gentle", "gently", "soft", "hint", "touch", "little", "bit", "mild", "faint", "light"];
const STRONGER: &[&str] = &["very", "strong", "strongly", "heavy", "heavily", "extreme", "extremely", "intense", "deep", "lots", "super", "really", "rich", "bold"];
const NEGATE: &[&str] = &["no", "without", "not", "zero", "none", "less"];
const STOP: &[&str] = &[
    "a", "an", "the", "and", "or", "with", "of", "in", "on", "at", "to", "for", "this", "that", "it", "its", "make", "look",
    "looks", "like", "feel", "feeling", "style", "scene", "photo", "image", "picture", "shot", "some", "more", "vibe", "vibes",
    "mood", "tone", "tones", "please", "give", "me", "my", "her", "his", "their", "your", "as", "is", "be", "from", "into",
    "kind", "sort", "effect", "everything", "overall", "just", "also", "but", "by", "set",
];

fn add(v: &mut f32, d: f32, lo: f32, hi: f32) {
    *v = (*v + d).clamp(lo, hi);
}

/// Add a tint to a colour wheel as a vector, so repeated or opposing tints combine.
fn tint(w: &mut ColorWheel, hue: f32, amount: f32, luminance: f32) {
    let (a, b) = (w.hue.to_radians(), hue.to_radians());
    let x = w.amount * a.cos() + amount * b.cos();
    let y = w.amount * a.sin() + amount * b.sin();
    let m = x.hypot(y);
    if m > 1e-3 {
        w.hue = y.atan2(x).to_degrees().rem_euclid(360.0);
    }
    w.amount = m.min(100.0);
    add(&mut w.luminance, luminance, -100.0, 100.0);
}

fn saturation(a: &mut Adjustments, d: f32) {
    for band in a.color.hsl.bands_mut() {
        add(&mut band.saturation, d, -100.0, 100.0);
    }
}

fn band<'a>(h: &'a mut HslBands, name: &str) -> &'a mut epikos_sidecar::HslChannel {
    let i = HslBands::NAMES.iter().position(|n| *n == name).expect("known band");
    h.bands_mut().into_iter().nth(i).expect("eight bands")
}

fn curve(a: &mut Adjustments, darks: f32, lights: f32, black: f32, white: f32) {
    let c = &mut a.curves.rgb;
    add(&mut c.darks, darks, -100.0, 100.0);
    add(&mut c.lights, lights, -100.0, 100.0);
    add(&mut c.black, black, -100.0, 100.0);
    add(&mut c.white, white, -100.0, 100.0);
}

fn split(a: &mut Adjustments, hh: f32, hs: f32, sh: f32, ss: f32) {
    let st = &mut a.split_toning;
    if hs != 0.0 {
        st.highlight_hue = hh;
        add(&mut st.highlight_saturation, hs, 0.0, 100.0);
    }
    if ss != 0.0 {
        st.shadow_hue = sh;
        add(&mut st.shadow_saturation, ss, 0.0, 100.0);
    }
}

const SHADOWS: usize = 0;
const MIDTONES: usize = 1;
const HIGHLIGHTS: usize = 2;

fn wheel(a: &mut Adjustments, which: usize, hue: f32, amount: f32, lum: f32) {
    let w = &mut a.color.wheels;
    let target = match which {
        SHADOWS => &mut w.shadows,
        MIDTONES => &mut w.midtones,
        _ => &mut w.highlights,
    };
    tint(target, hue, amount, lum);
}

fn concepts() -> Vec<Concept> {
    macro_rules! c {
        ([$($p:literal),+], $effect:literal, |$a:ident, $k:ident| $body:block) => {
            Concept { phrases: &[$($p),+], effect: $effect, apply: |$a: &mut Adjustments, $k: f32| $body }
        };
    }
    vec![
        // Light and time of day (longest phrases first where they overlap).
        c!(["golden hour", "sunset", "sunrise", "golden light", "low sun", "warm light", "magic hour"], "warm low-sun light with glow and rays", |a, k| {
            wheel(a, HIGHLIGHTS, 42.0, 30.0 * k, 3.0 * k);
            wheel(a, MIDTONES, 38.0, 12.0 * k, 0.0);
            add(&mut a.atmosphere.glow, 30.0 * k, 0.0, 100.0);
            a.atmosphere.glow_warmth = 80.0;
            add(&mut a.atmosphere.shafts, 20.0 * k, 0.0, 100.0);
            a.atmosphere.shaft_warmth = 80.0;
        }),
        c!(["blue hour", "twilight", "dusk", "dawn"], "cool twilight tones", |a, k| {
            wheel(a, SHADOWS, 225.0, 25.0 * k, 0.0);
            wheel(a, MIDTONES, 230.0, 12.0 * k, 0.0);
            add(&mut a.exposure, -0.2 * k, -5.0, 5.0);
        }),
        c!(["night", "nighttime", "midnight", "nocturnal"], "darker, cool night", |a, k| {
            add(&mut a.exposure, -0.6 * k, -5.0, 5.0);
            wheel(a, SHADOWS, 225.0, 20.0 * k, 0.0);
            curve(a, 0.0, 0.0, -10.0 * k, 0.0);
        }),
        c!(["god rays", "light rays", "sun rays", "sunbeam", "sunbeams", "crepuscular", "volumetric light", "light shafts", "rays"], "volumetric light shafts", |a, k| {
            add(&mut a.atmosphere.shafts, 55.0 * k, 0.0, 100.0);
            a.atmosphere.shaft_warmth = 60.0;
        }),
        c!(["glow", "glowing", "bloom", "blooming", "luminous"], "soft glow around the lights", |a, k| {
            add(&mut a.atmosphere.glow, 35.0 * k, 0.0, 100.0);
        }),
        // Weather.
        c!(["foggy", "fog", "misty", "mist", "hazy", "haze", "atmospheric"], "distance fog", |a, k| {
            add(&mut a.atmosphere.fog, 45.0 * k, 0.0, 100.0);
            a.atmosphere.fog_start = 20.0;
            add(&mut a.atmosphere.glow, 8.0 * k, 0.0, 100.0);
        }),
        c!(["smoky", "smoke", "smokey"], "smoky haze", |a, k| {
            add(&mut a.atmosphere.fog, 30.0 * k, 0.0, 100.0);
            a.atmosphere.fog_warmth = -10.0;
            saturation(a, -10.0 * k);
        }),
        c!(["rainy", "rain", "stormy", "storm", "wet"], "cool, muted rainy light", |a, k| {
            saturation(a, -20.0 * k);
            wheel(a, SHADOWS, 210.0, 15.0 * k, 0.0);
            add(&mut a.texture.clarity, 10.0 * k, -100.0, 100.0);
            add(&mut a.exposure, -0.2 * k, -5.0, 5.0);
        }),
        c!(["snowy", "snow", "winter", "wintry", "icy", "frosty", "frost"], "bright, cool winter light", |a, k| {
            wheel(a, HIGHLIGHTS, 200.0, 10.0 * k, 0.0);
            add(&mut a.exposure, 0.2 * k, -5.0, 5.0);
            saturation(a, -10.0 * k);
        }),
        c!(["overcast", "gloomy", "grey", "gray", "cloudy"], "flat, cool overcast", |a, k| {
            saturation(a, -15.0 * k);
            curve(a, 0.0, -10.0 * k, 0.0, 0.0);
            wheel(a, MIDTONES, 210.0, 8.0 * k, 0.0);
        }),
        c!(["sunny", "summer", "summery", "sunlit"], "warm sunny colour", |a, k| {
            wheel(a, HIGHLIGHTS, 45.0, 10.0 * k, 0.0);
            saturation(a, 10.0 * k);
        }),
        // Moods.
        c!(["eerie", "creepy", "haunting", "haunted", "ominous", "spooky", "sinister", "unsettling"], "eerie: drained colour, cold green shadows", |a, k| {
            saturation(a, -25.0 * k);
            wheel(a, SHADOWS, 165.0, 20.0 * k, -8.0 * k);
            wheel(a, MIDTONES, 150.0, 8.0 * k, 0.0);
            curve(a, -10.0 * k, 0.0, 0.0, 0.0);
            add(&mut a.finishing.vignette, -25.0 * k, -100.0, 100.0);
        }),
        c!(["moody", "brooding", "melancholic", "melancholy", "somber", "sombre"], "moody: darker, muted, vignetted", |a, k| {
            add(&mut a.exposure, -0.2 * k, -5.0, 5.0);
            curve(a, -15.0 * k, 5.0 * k, 0.0, 0.0);
            saturation(a, -15.0 * k);
            add(&mut a.finishing.vignette, -20.0 * k, -100.0, 100.0);
            add(&mut a.texture.clarity, 10.0 * k, -100.0, 100.0);
        }),
        c!(["dreamy", "ethereal", "dreamlike", "whimsical", "soft focus"], "dreamy: glow, softness, lifted blacks", |a, k| {
            add(&mut a.atmosphere.glow, 35.0 * k, 0.0, 100.0);
            a.atmosphere.glow_warmth = 20.0;
            add(&mut a.texture.clarity, -25.0 * k, -100.0, 100.0);
            curve(a, 0.0, 0.0, 20.0 * k, 0.0);
            wheel(a, HIGHLIGHTS, 35.0, 8.0 * k, 0.0);
        }),
        c!(["cinematic", "movie", "hollywood", "blockbuster", "filmic"], "cinematic teal and orange, matte blacks", |a, k| {
            wheel(a, SHADOWS, 190.0, 22.0 * k, 0.0);
            wheel(a, HIGHLIGHTS, 35.0, 18.0 * k, 0.0);
            curve(a, -10.0 * k, 8.0 * k, 12.0 * k, 0.0);
            add(&mut a.finishing.vignette, -15.0 * k, -100.0, 100.0);
        }),
        c!(["dramatic", "epic", "intense", "powerful"], "dramatic contrast and structure", |a, k| {
            add(&mut a.texture.clarity, 30.0 * k, -100.0, 100.0);
            add(&mut a.texture.micro_texture, 15.0 * k, -100.0, 100.0);
            curve(a, -20.0 * k, 15.0 * k, 0.0, 0.0);
            add(&mut a.finishing.vignette, -30.0 * k, -100.0, 100.0);
        }),
        c!(["romantic", "tender", "intimate"], "warm, soft and glowing", |a, k| {
            wheel(a, HIGHLIGHTS, 30.0, 15.0 * k, 0.0);
            add(&mut a.atmosphere.glow, 20.0 * k, 0.0, 100.0);
            add(&mut a.texture.clarity, -10.0 * k, -100.0, 100.0);
        }),
        c!(["gritty", "grungy", "raw", "harsh", "edgy"], "gritty texture, muted colour, grain", |a, k| {
            add(&mut a.texture.micro_texture, 40.0 * k, -100.0, 100.0);
            add(&mut a.texture.clarity, 25.0 * k, -100.0, 100.0);
            saturation(a, -20.0 * k);
            add(&mut a.finishing.grain, 30.0 * k, 0.0, 100.0);
        }),
        c!(["crisp", "clean", "sharp", "pristine"], "crisp detail", |a, k| {
            add(&mut a.texture.micro_texture, 20.0 * k, -100.0, 100.0);
            add(&mut a.texture.clarity, 10.0 * k, -100.0, 100.0);
        }),
        c!(["light and airy", "airy", "bright", "high key", "high-key"], "bright and airy", |a, k| {
            add(&mut a.exposure, 0.4 * k, -5.0, 5.0);
            add(&mut a.curves.rgb.shadows, 15.0 * k, -100.0, 100.0);
            curve(a, 0.0, 0.0, 8.0 * k, 0.0);
            saturation(a, -8.0 * k);
        }),
        c!(["low key", "low-key", "dark", "shadowy"], "low key: darker with deep blacks", |a, k| {
            add(&mut a.exposure, -0.5 * k, -5.0, 5.0);
            curve(a, 0.0, 0.0, -20.0 * k, 0.0);
            add(&mut a.curves.rgb.shadows, -15.0 * k, -100.0, 100.0);
            add(&mut a.finishing.vignette, -20.0 * k, -100.0, 100.0);
        }),
        c!(["vibrant", "punchy", "colorful", "colourful", "vivid", "saturated", "poppy"], "vibrant colour and contrast", |a, k| {
            saturation(a, 20.0 * k);
            curve(a, -8.0 * k, 8.0 * k, 0.0, 0.0);
        }),
        c!(["muted", "desaturated", "washed out", "faded", "subdued", "understated"], "muted, faded colour", |a, k| {
            saturation(a, -25.0 * k);
            curve(a, 0.0, 0.0, 20.0 * k, -15.0 * k);
        }),
        c!(["matte", "lifted blacks"], "matte black point", |a, k| {
            curve(a, 0.0, 0.0, 30.0 * k, 0.0);
        }),
        c!(["high contrast", "contrasty"], "high contrast", |a, k| {
            curve(a, -20.0 * k, 20.0 * k, 0.0, 0.0);
            add(&mut a.texture.clarity, 10.0 * k, -100.0, 100.0);
        }),
        c!(["low contrast", "flat", "soft contrast"], "low contrast", |a, k| {
            curve(a, 12.0 * k, -12.0 * k, 0.0, 0.0);
        }),
        // Eras and film.
        c!(["1970s", "70s", "seventies", "1970"], "1970s: warm faded film, olive shadows, grain", |a, k| {
            split(a, 40.0, 30.0 * k, 80.0, 20.0 * k);
            curve(a, 0.0, 0.0, 25.0 * k, -8.0 * k);
            add(&mut a.finishing.grain, 35.0 * k, 0.0, 100.0);
            add(&mut band(&mut a.color.hsl, "yellow").saturation, 15.0 * k, -100.0, 100.0);
            add(&mut band(&mut a.color.hsl, "orange").saturation, 10.0 * k, -100.0, 100.0);
            add(&mut band(&mut a.color.hsl, "blue").saturation, -20.0 * k, -100.0, 100.0);
        }),
        c!(["1980s", "80s", "eighties", "1980"], "1980s: magenta highlights, cyan shadows", |a, k| {
            split(a, 320.0, 20.0 * k, 190.0, 25.0 * k);
            curve(a, -10.0 * k, 10.0 * k, 0.0, 0.0);
            saturation(a, 10.0 * k);
        }),
        c!(["1990s", "90s", "nineties", "1990"], "1990s: green-tinged shadows, grain", |a, k| {
            split(a, 0.0, 0.0, 120.0, 15.0 * k);
            add(&mut a.finishing.grain, 25.0 * k, 0.0, 100.0);
            curve(a, 0.0, 0.0, 10.0 * k, 0.0);
        }),
        c!(["vintage", "retro", "nostalgic", "old photo", "old-fashioned", "timeless"], "vintage fade, split tone and grain", |a, k| {
            curve(a, 0.0, 0.0, 25.0 * k, -10.0 * k);
            split(a, 45.0, 25.0 * k, 180.0, 20.0 * k);
            add(&mut a.finishing.grain, 30.0 * k, 0.0, 100.0);
            saturation(a, -10.0 * k);
        }),
        c!(["kodak portra", "portra"], "Portra: soft contrast, warm skin", |a, k| {
            curve(a, 8.0 * k, -5.0 * k, 10.0 * k, 0.0);
            add(&mut band(&mut a.color.hsl, "orange").luminance, 5.0 * k, -100.0, 100.0);
            wheel(a, HIGHLIGHTS, 35.0, 10.0 * k, 0.0);
            add(&mut a.finishing.grain, 20.0 * k, 0.0, 100.0);
        }),
        c!(["velvia", "fuji velvia"], "Velvia: saturated greens and blues", |a, k| {
            saturation(a, 20.0 * k);
            add(&mut band(&mut a.color.hsl, "green").saturation, 20.0 * k, -100.0, 100.0);
            add(&mut band(&mut a.color.hsl, "blue").saturation, 15.0 * k, -100.0, 100.0);
            curve(a, -10.0 * k, 5.0 * k, 0.0, 0.0);
        }),
        c!(["film", "analog", "analogue", "35mm", "kodak", "filmstock", "film stock"], "film: grain, soft blacks, warm highlights", |a, k| {
            add(&mut a.finishing.grain, 30.0 * k, 0.0, 100.0);
            curve(a, 0.0, 0.0, 12.0 * k, 0.0);
            split(a, 35.0, 15.0 * k, 0.0, 0.0);
        }),
        c!(["black and white", "black & white", "b&w", "bw", "monochrome", "monochromatic", "grayscale", "greyscale", "tri-x", "hp5"], "black and white", |a, k| {
            if k > 0.0 {
                for b in a.color.hsl.bands_mut() {
                    b.saturation = -100.0;
                }
                curve(a, -10.0 * k, 10.0 * k, 0.0, 0.0);
            } else {
                for b in a.color.hsl.bands_mut() {
                    b.saturation = b.saturation.max(0.0);
                }
            }
        }),
        c!(["noir", "film noir"], "noir: deep shadows and vignette", |a, k| {
            curve(a, -20.0 * k, 10.0 * k, -15.0 * k, 0.0);
            add(&mut a.finishing.vignette, -40.0 * k, -100.0, 100.0);
            saturation(a, -40.0 * k);
        }),
        // Places.
        c!(["scandinavian", "nordic", "norwegian", "swedish", "danish", "finnish", "icelandic"], "Scandinavian: cool, restrained colour", |a, k| {
            wheel(a, SHADOWS, 210.0, 15.0 * k, 0.0);
            wheel(a, HIGHLIGHTS, 200.0, 5.0 * k, 0.0);
            saturation(a, -20.0 * k);
            add(&mut band(&mut a.color.hsl, "green").saturation, -15.0 * k, -100.0, 100.0);
            curve(a, 0.0, 0.0, 10.0 * k, 0.0);
            add(&mut a.texture.clarity, 5.0 * k, -100.0, 100.0);
        }),
        c!(["mediterranean", "italian", "greek", "riviera"], "Mediterranean: warm light, deep blues", |a, k| {
            wheel(a, HIGHLIGHTS, 40.0, 15.0 * k, 0.0);
            add(&mut band(&mut a.color.hsl, "blue").saturation, 20.0 * k, -100.0, 100.0);
            add(&mut band(&mut a.color.hsl, "orange").saturation, 10.0 * k, -100.0, 100.0);
        }),
        c!(["tropical", "jungle", "lush"], "tropical: lush greens and aquas", |a, k| {
            add(&mut band(&mut a.color.hsl, "green").saturation, 20.0 * k, -100.0, 100.0);
            add(&mut band(&mut a.color.hsl, "aqua").saturation, 25.0 * k, -100.0, 100.0);
            saturation(a, 5.0 * k);
        }),
        c!(["desert", "arid", "dusty", "sahara"], "desert warmth", |a, k| {
            wheel(a, HIGHLIGHTS, 35.0, 25.0 * k, 0.0);
            add(&mut band(&mut a.color.hsl, "orange").saturation, 15.0 * k, -100.0, 100.0);
            add(&mut band(&mut a.color.hsl, "blue").saturation, -15.0 * k, -100.0, 100.0);
        }),
        c!(["urban", "street", "city", "industrial"], "urban: structure, muted colour", |a, k| {
            add(&mut a.texture.clarity, 15.0 * k, -100.0, 100.0);
            saturation(a, -15.0 * k);
            curve(a, -8.0 * k, 8.0 * k, 0.0, 0.0);
        }),
        // Colour words.
        c!(["golden", "gold", "warm", "warmer", "amber"], "warmer highlights", |a, k| {
            wheel(a, HIGHLIGHTS, 40.0, 15.0 * k, 0.0);
            wheel(a, MIDTONES, 38.0, 6.0 * k, 0.0);
        }),
        c!(["cool", "cold", "cooler", "blue", "bluish", "icy blue"], "cooler tones", |a, k| {
            wheel(a, SHADOWS, 210.0, 15.0 * k, 0.0);
            wheel(a, HIGHLIGHTS, 205.0, 8.0 * k, 0.0);
        }),
        c!(["teal"], "teal shadows", |a, k| { wheel(a, SHADOWS, 185.0, 20.0 * k, 0.0); }),
        c!(["orange"], "orange highlights", |a, k| { wheel(a, HIGHLIGHTS, 30.0, 12.0 * k, 0.0); }),
        c!(["pink", "pastel", "rosy", "blush"], "soft pastel pink", |a, k| {
            wheel(a, HIGHLIGHTS, 340.0, 10.0 * k, 0.0);
            curve(a, 0.0, 0.0, 10.0 * k, 0.0);
            saturation(a, -8.0 * k);
        }),
        c!(["green", "greenish", "emerald"], "green tint", |a, k| { wheel(a, MIDTONES, 120.0, 10.0 * k, 0.0); }),
        c!(["purple", "violet", "magenta", "lavender"], "purple shadows", |a, k| { wheel(a, SHADOWS, 285.0, 15.0 * k, 0.0); }),
        // Skin, texture, finishing.
        c!(["glowing skin", "skin glow", "luminous skin", "dewy skin", "radiant skin"], "luminous skin", |a, k| {
            add(&mut band(&mut a.color.hsl, "orange").luminance, 12.0 * k, -100.0, 100.0);
            add(&mut a.texture.specular_balance, 20.0 * k, 0.0, 100.0);
            add(&mut a.atmosphere.glow, 10.0 * k, 0.0, 100.0);
        }),
        c!(["smooth skin", "flawless", "retouched", "clean skin", "soft skin"], "smoother skin", |a, k| {
            add(&mut a.texture.blemish_smoothing, 50.0 * k, 0.0, 100.0);
            add(&mut a.texture.specular_balance, 25.0 * k, 0.0, 100.0);
        }),
        c!(["texture", "textured", "detail", "detailed", "details"], "more micro-texture", |a, k| {
            add(&mut a.texture.micro_texture, 35.0 * k, -100.0, 100.0);
            add(&mut a.texture.clarity, 10.0 * k, -100.0, 100.0);
        }),
        c!(["grain", "grainy", "film grain", "noise"], "film grain", |a, k| {
            add(&mut a.finishing.grain, 45.0 * k, 0.0, 100.0);
        }),
        c!(["vignette", "vignetted", "dark corners", "dark edges"], "edge vignette", |a, k| {
            add(&mut a.finishing.vignette, -35.0 * k, -100.0, 100.0);
        }),
    ]
}

/// "<colour?> light on the face" and backlight phrases, handled before the general
/// vocabulary because they place lights rather than change the grade.
const FACE_NOUNS: &[&str] = &["face", "faces", "subject", "model", "person", "portrait", "skin", "her", "him", "them"];

/// Interpret `prompt` against `base`. `subject` is the subject's position (0–1 across,
/// down) for placing lights, e.g. the centre of the detected skin.
pub fn interpret_look(prompt: &str, base: &Adjustments, subject: Option<(f32, f32)>) -> LookPrompt {
    let words: Vec<String> = prompt
        .to_lowercase()
        .replace(['&'], " & ")
        .split(|c: char| !(c.is_alphanumeric() || c == '&' || c == '-'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    let mut used = vec![false; words.len()];
    let mut out = base.clone();
    let mut matched: Vec<(usize, PromptMatch)> = Vec::new();
    let (sx, sy) = subject.unwrap_or((0.5, 0.4));

    // Strength from the two words before position `i` (skipping stop words).
    let strength_before = |i: usize, used: &mut Vec<bool>| -> f32 {
        let mut k = 1.0;
        let mut j = i;
        let mut seen = 0;
        while j > 0 && seen < 3 {
            j -= 1;
            let w = words[j].as_str();
            if SOFTER.contains(&w) {
                k *= 0.5;
                used[j] = true;
            } else if STRONGER.contains(&w) {
                k *= 1.6;
                used[j] = true;
            } else if NEGATE.contains(&w) {
                k = -1.0;
                used[j] = true;
            } else if !STOP.contains(&w) {
                break;
            }
            seen += 1;
        }
        k
    };

    // Lights: "[soft] [golden] light on the face", "rim light", "backlit".
    for i in 0..words.len() {
        if used[i] || !matches!(words[i].as_str(), "light" | "lighting" | "spotlight" | "glow") {
            continue;
        }
        let after: Vec<&str> = words[i + 1..].iter().take(4).map(String::as_str).collect();
        let on_face = after.first().is_some_and(|w| matches!(*w, "on" | "across" | "over"))
            && after.iter().skip(1).take(3).any(|w| FACE_NOUNS.contains(w));
        if !on_face {
            continue;
        }
        let mut start = i;
        let mut warmth = 30.0;
        let mut colour = "";
        if i > 0 {
            let prev = words[i - 1].as_str();
            let hue = match prev {
                "golden" | "gold" | "warm" | "amber" | "sunny" => Some(75.0),
                "cool" | "cold" | "blue" | "moonlit" => Some(-45.0),
                "soft" | "white" | "neutral" => Some(0.0),
                _ => None,
            };
            if let Some(h) = hue {
                warmth = h;
                colour = prev;
                start = i - 1;
            }
        }
        let k = strength_before(start, &mut used).max(0.0);
        let end = i + 1 + after.iter().position(|w| FACE_NOUNS.contains(w)).unwrap_or(0) + 1;
        (start..end.min(words.len())).for_each(|j| used[j] = true);
        out.atmosphere.lights.push(VirtualLight {
            x: sx,
            y: sy,
            depth: 0.15,
            intensity: (45.0 * k).clamp(5.0, 100.0),
            reach: 40.0,
            warmth,
            halo: 5.0,
        });
        matched.push((
            start,
            PromptMatch {
                phrase: words[start..end.min(words.len())].join(" "),
                effect: format!(
                    "{}light placed on the subject",
                    if colour.is_empty() { String::new() } else { format!("{colour} ") }
                ),
                strength: k,
            },
        ));
    }
    for i in 0..words.len() {
        if used[i] {
            continue;
        }
        let two = words.get(i + 1).map(|n| format!("{} {}", words[i], n));
        let rim = matches!(words[i].as_str(), "backlit" | "backlight" | "backlighting" | "silhouette")
            || two.as_deref().is_some_and(|t| matches!(t, "rim light" | "rim lighting" | "back light"));
        if !rim {
            continue;
        }
        let len = if matches!(words[i].as_str(), "rim" | "back") { 2 } else { 1 };
        let k = strength_before(i, &mut used).max(0.0);
        (i..(i + len).min(words.len())).for_each(|j| used[j] = true);
        out.atmosphere.lights.push(VirtualLight {
            x: sx,
            y: (sy - 0.1).max(0.0),
            depth: 0.85,
            intensity: (70.0 * k).clamp(5.0, 100.0),
            reach: 55.0,
            warmth: 50.0,
            halo: 60.0,
        });
        matched.push((
            i,
            PromptMatch { phrase: words[i..(i + len).min(words.len())].join(" "), effect: "rim light behind the subject".into(), strength: k },
        ));
    }

    // General vocabulary: longest phrase first at each position; each concept once.
    let vocabulary = concepts();
    let mut applied = vec![false; vocabulary.len()];
    let mut i = 0;
    while i < words.len() {
        if used[i] {
            i += 1;
            continue;
        }
        let mut best: Option<(usize, usize)> = None; // (concept, phrase length)
        for (ci, c) in vocabulary.iter().enumerate() {
            for p in c.phrases {
                let n = p.split(' ').count();
                if i + n <= words.len()
                    && (i..i + n).all(|j| !used[j])
                    && words[i..i + n].join(" ") == *p
                    && best.is_none_or(|(_, bn)| n > bn)
                {
                    best = Some((ci, n));
                }
            }
        }
        match best {
            Some((ci, n)) => {
                let k = strength_before(i, &mut used);
                (i..i + n).for_each(|j| used[j] = true);
                if !applied[ci] {
                    applied[ci] = true;
                    (vocabulary[ci].apply)(&mut out, k);
                    matched.push((
                        i,
                        PromptMatch { phrase: words[i..i + n].join(" "), effect: vocabulary[ci].effect.into(), strength: k },
                    ));
                }
                i += n;
            }
            None => i += 1,
        }
    }

    matched.sort_by_key(|(pos, _)| *pos);
    let unknown = words
        .iter()
        .zip(&used)
        .filter(|(w, u)| !**u && !STOP.contains(&w.as_str()) && !SOFTER.contains(&w.as_str()) && !STRONGER.contains(&w.as_str()) && !NEGATE.contains(&w.as_str()))
        .map(|(w, _)| w.clone())
        .collect();
    LookPrompt { adjustments: out, matched: matched.into_iter().map(|(_, m)| m).collect(), unknown }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prd_example_becomes_real_settings() {
        let base = Adjustments::default();
        let r = interpret_look(
            "Make this look like an eerie, foggy 1970s Scandinavian film scene with subtle golden light on the face",
            &base,
            Some((0.45, 0.35)),
        );
        let effects: Vec<&str> = r.matched.iter().map(|m| m.phrase.as_str()).collect();
        assert_eq!(effects, ["eerie", "foggy", "1970s", "scandinavian", "film", "golden light on the face"], "{:?}", r.matched);
        assert!(r.unknown.is_empty(), "{:?}", r.unknown);
        let a = &r.adjustments;
        assert!(a.atmosphere.fog > 30.0);
        assert!(a.finishing.grain > 30.0, "1970s + film grain");
        assert!(a.color.hsl.green.saturation < -20.0, "eerie + Scandinavian drain colour");
        let light = a.atmosphere.lights.last().expect("a light on the face");
        assert_eq!((light.x, light.y), (0.45, 0.35));
        assert!(light.warmth > 50.0 && light.intensity < 30.0, "golden and subtle: {light:?}");
    }

    #[test]
    fn intensity_and_negation_words_work() {
        let base = Adjustments { finishing: epikos_sidecar::Finishing { grain: 40.0, ..Default::default() }, ..Default::default() };
        let r = interpret_look("very dramatic, no grain", &base, None);
        assert_eq!(r.adjustments.finishing.grain, 0.0);
        let plain = interpret_look("dramatic", &Adjustments::default(), None);
        assert!(r.adjustments.texture.clarity > plain.adjustments.texture.clarity);
        assert!((r.matched[0].strength - 1.6).abs() < 1e-6);
    }

    #[test]
    fn unknown_words_are_reported_not_guessed() {
        let r = interpret_look("zorblax moody", &Adjustments::default(), None);
        assert_eq!(r.unknown, ["zorblax"]);
        assert_eq!(r.matched.len(), 1);
    }

    #[test]
    fn black_and_white_is_one_phrase() {
        let r = interpret_look("black and white noir", &Adjustments::default(), None);
        assert!(r.adjustments.color.hsl.red.saturation <= -99.0);
        assert_eq!(r.matched.len(), 2);
    }
}
