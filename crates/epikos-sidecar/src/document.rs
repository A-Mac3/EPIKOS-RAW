use serde::{Deserialize, Serialize};

/// Canonical develop document. Round-trips 1:1 through JSON and XMP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopDocument {
    pub version: u32,
    pub source: SourceRef,
    pub adjustments: Adjustments,
}

impl DevelopDocument {
    pub const VERSION: u32 = 1;

    pub fn new(source: SourceRef) -> Self {
        Self {
            version: Self::VERSION,
            source,
            adjustments: Adjustments::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub path: String,
    pub sha256: String,
    pub format: String,
    pub make: String,
    pub model: String,
}

/// Missing fields take their default, so sidecars written by older versions still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Adjustments {
    pub white_balance: WhiteBalance,
    /// Reconstruct clipped CFA channels from unclipped neighbours.
    pub highlight_recovery: bool,
    pub demosaic: DemosaicMode,
    pub lens: LensCorrections,
    /// Global exposure in stops (EV), applied in scene-referred linear light.
    pub exposure: f32,
    /// Step 2: contrast, highlights, shadows, whites, blacks, vibrance, saturation.
    pub tone: Tone,
    pub noise_reduction: NoiseReduction,
    /// Step 3: edits confined to an AI mask.
    pub local: Vec<LocalAdjustment>,
    /// Step 4: micro-texture and skin retouching.
    pub texture: Texture,
    /// Step 5: HSL and three-way colour grading.
    pub color: ColorGrade,
    /// Step 6: glow, depth-based fog and light shafts.
    pub atmosphere: Atmosphere,
    /// Step 7: parametric tone curves.
    pub curves: Curves,
    /// Step 7: split toning.
    pub split_toning: SplitToning,
    /// Step 8: film grain and vignette.
    pub finishing: Finishing,
    /// Parametric style layered on top of the manual Step 4–6 settings.
    pub style: StyleRef,
    /// An imported 3D LUT, applied after Step 7.
    pub lut: LutRef,
    /// Step 1 crop of the upright (straightened) frame.
    pub crop: Crop,
    /// Step 3 hand-drawn masks (brush, linear and radial gradients) with their edits.
    pub manual: Vec<ManualAdjustment>,
}

impl Default for Adjustments {
    fn default() -> Self {
        Self {
            white_balance: WhiteBalance::default(),
            highlight_recovery: true,
            demosaic: DemosaicMode::Auto,
            lens: LensCorrections::default(),
            exposure: 0.0,
            tone: Tone::default(),
            noise_reduction: NoiseReduction::default(),
            local: Vec::new(),
            texture: Texture::default(),
            color: ColorGrade::default(),
            atmosphere: Atmosphere::default(),
            curves: Curves::default(),
            split_toning: SplitToning::default(),
            finishing: Finishing::default(),
            style: StyleRef::default(),
            lut: LutRef::default(),
            crop: Crop::default(),
            manual: Vec::new(),
        }
    }
}

/// An imported `.cube` LUT by name (the file name in the app's LUT folder), blended at
/// `amount` 0…100. An empty name is none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LutRef {
    pub name: String,
    pub amount: f32,
}

impl Default for LutRef {
    fn default() -> Self {
        Self { name: String::new(), amount: 100.0 }
    }
}

impl LutRef {
    pub fn is_none(&self) -> bool {
        self.name.is_empty() || self.amount <= 0.0
    }
}

/// PRD Step 2 tone controls, each −100…100, zero is neutral. Highlights and shadows
/// work on a smoothed luminance so local contrast survives; whites and blacks move
/// the ends of the range; vibrance favours muted colours and spares skin.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Tone {
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub vibrance: f32,
    pub saturation: f32,
    /// −100…100: removes (+) or adds (−) atmospheric haze (dark-channel prior).
    pub dehaze: f32,
}

impl Tone {
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }

    pub fn to_array(&self) -> [f32; 7] {
        [self.contrast, self.highlights, self.shadows, self.whites, self.blacks, self.vibrance, self.saturation]
    }

    /// The seven original sliders; [`Tone::dehaze`] is kept separately.
    pub fn from_array([contrast, highlights, shadows, whites, blacks, vibrance, saturation]: [f32; 7]) -> Self {
        Self { contrast, highlights, shadows, whites, blacks, vibrance, saturation, dehaze: 0.0 }
    }
}

/// A region from Step 3's models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MaskTarget {
    /// The main subject (IS-Net).
    #[default]
    Subject,
    /// Everything but the subject.
    Background,
    Sky,
    /// Skin anywhere in the frame (colour model).
    Skin,
    /// Eyes (face parsing).
    Eyes,
    /// Hair on the head (face parsing).
    Hair,
    /// The near part of the scene (depth model).
    Foreground,
    /// Eyebrows (face parsing).
    Eyebrows,
    /// The lash line: dark detail just around the eyes (from the eye mask; there is no
    /// lash class in the face model).
    Eyelashes,
    /// Teeth: the light part of the inside of the mouth, lips excluded.
    Teeth,
    /// Skin of the face (face parsing): cheeks, forehead, nose, ears.
    FaceSkin,
    /// Skin other than the face: neck, arms, hands.
    BodySkin,
}

impl MaskTarget {
    pub const ALL: [MaskTarget; 12] = [
        MaskTarget::Subject,
        MaskTarget::Background,
        MaskTarget::Sky,
        MaskTarget::Skin,
        MaskTarget::Eyes,
        MaskTarget::Hair,
        MaskTarget::Foreground,
        MaskTarget::Eyebrows,
        MaskTarget::Eyelashes,
        MaskTarget::Teeth,
        MaskTarget::FaceSkin,
        MaskTarget::BodySkin,
    ];

    pub fn id(self) -> &'static str {
        match self {
            MaskTarget::Subject => "subject",
            MaskTarget::Background => "background",
            MaskTarget::Sky => "sky",
            MaskTarget::Skin => "skin",
            MaskTarget::Eyes => "eyes",
            MaskTarget::Hair => "hair",
            MaskTarget::Foreground => "foreground",
            MaskTarget::Eyebrows => "eyebrows",
            MaskTarget::Eyelashes => "eyelashes",
            MaskTarget::Teeth => "teeth",
            MaskTarget::FaceSkin => "faceSkin",
            MaskTarget::BodySkin => "bodySkin",
        }
    }

    /// Face features: made by the face-parsing model.
    pub fn is_face_feature(self) -> bool {
        matches!(
            self,
            MaskTarget::Eyes | MaskTarget::Hair | MaskTarget::Eyebrows | MaskTarget::Eyelashes | MaskTarget::Teeth | MaskTarget::FaceSkin
        )
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            MaskTarget::Subject => "Subject",
            MaskTarget::Background => "Background",
            MaskTarget::Sky => "Sky",
            MaskTarget::Skin => "Skin",
            MaskTarget::Eyes => "Eyes",
            MaskTarget::Hair => "Hair",
            MaskTarget::Foreground => "Foreground",
            MaskTarget::Eyebrows => "Eyebrows",
            MaskTarget::Eyelashes => "Eyelashes",
            MaskTarget::Teeth => "Teeth",
            MaskTarget::FaceSkin => "Facial skin",
            MaskTarget::BodySkin => "Body skin",
        }
    }
}

/// PRD Step 3: an adjustment applied only inside a mask. Exposure in EV (−3…3), the
/// rest −100…100.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalAdjustment {
    pub mask: MaskTarget,
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    /// Cooler (−) or warmer (+).
    pub warmth: f32,
    /// Mid-scale local contrast.
    pub clarity: f32,
    /// Greener (−) or more magenta (+): balances a skin cast the warmth axis can't.
    pub tint: f32,
}

impl LocalAdjustment {
    pub fn is_neutral(&self) -> bool {
        self.exposure == 0.0
            && self.contrast == 0.0
            && self.saturation == 0.0
            && self.warmth == 0.0
            && self.clarity == 0.0
            && self.tint == 0.0
    }
}

/// A hand-drawn mask. Positions are fractions of the upright (uncropped) frame; sizes
/// along x are fractions of its width, along y of its height.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ManualShape {
    /// Full effect on the (x0, y0) side, fading to none at (x1, y1): sky or ground.
    #[serde(rename_all = "camelCase")]
    Linear { x0: f32, y0: f32, x1: f32, y1: f32 },
    /// An ellipse centred at (cx, cy) with radii rx, ry, turned `angle` degrees;
    /// `feather` 0…100 softens it from the edge inwards. `invert` edits the outside.
    #[serde(rename_all = "camelCase")]
    Radial { cx: f32, cy: f32, rx: f32, ry: f32, angle: f32, feather: f32, invert: bool },
    /// Painted strokes.
    #[serde(rename_all = "camelCase")]
    Brush { strokes: Vec<BrushStroke> },
}

/// One brush stroke: its path, size (fraction of the frame width), feather and flow
/// (0…100); an erase stroke removes mask instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrushStroke {
    pub points: Vec<[f32; 2]>,
    pub size: f32,
    pub feather: f32,
    pub flow: f32,
    pub erase: bool,
}

impl Default for BrushStroke {
    fn default() -> Self {
        Self { points: Vec::new(), size: 0.05, feather: 50.0, flow: 100.0, erase: false }
    }
}

/// A local edit through a hand-drawn mask: exposure (EV), contrast, saturation,
/// warmth (temperature), tint, clarity and dehaze (−100…100).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManualAdjustment {
    pub shape: ManualShape,
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub warmth: f32,
    pub tint: f32,
    pub clarity: f32,
    pub dehaze: f32,
}

impl Default for ManualAdjustment {
    fn default() -> Self {
        Self {
            shape: ManualShape::Radial { cx: 0.5, cy: 0.5, rx: 0.25, ry: 0.25, angle: 0.0, feather: 50.0, invert: false },
            exposure: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            warmth: 0.0,
            tint: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
        }
    }
}

impl ManualAdjustment {
    pub fn is_neutral(&self) -> bool {
        [self.exposure, self.contrast, self.saturation, self.warmth, self.tint, self.clarity, self.dehaze]
            .iter()
            .all(|v| *v == 0.0)
    }

    /// The same edit on a crop of the frame: positions and sizes in the crop's own
    /// fractions.
    pub fn cropped(&self, c: &Crop) -> ManualAdjustment {
        let c = c.clamped();
        let (fx, fy) = (|x: f32| (x - c.x) / c.width, |y: f32| (y - c.y) / c.height);
        let shape = match &self.shape {
            ManualShape::Linear { x0, y0, x1, y1 } => ManualShape::Linear { x0: fx(*x0), y0: fy(*y0), x1: fx(*x1), y1: fy(*y1) },
            ManualShape::Radial { cx, cy, rx, ry, angle, feather, invert } => ManualShape::Radial {
                cx: fx(*cx),
                cy: fy(*cy),
                rx: rx / c.width,
                ry: ry / c.height,
                angle: *angle,
                feather: *feather,
                invert: *invert,
            },
            ManualShape::Brush { strokes } => ManualShape::Brush {
                strokes: strokes
                    .iter()
                    .map(|s| BrushStroke {
                        points: s.points.iter().map(|[x, y]| [fx(*x), fy(*y)]).collect(),
                        size: s.size / c.width,
                        ..s.clone()
                    })
                    .collect(),
            },
        };
        ManualAdjustment { shape, ..self.clone() }
    }
}

/// A crop of the upright frame (after straighten and perspective), as fractions of its
/// width and height. The default is the whole frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// The aspect preset it was drawn with ("free", "original", "1:1", "4:5", "16:9",
    /// "9:16"), so the crop tool reopens with it.
    pub aspect: String,
}

impl Default for Crop {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, width: 1.0, height: 1.0, aspect: "free".into() }
    }
}

impl Crop {
    /// Whether it cuts anything away.
    pub fn is_active(&self) -> bool {
        let c = self.clamped();
        c.x > 1e-4 || c.y > 1e-4 || c.width < 1.0 - 1e-4 || c.height < 1.0 - 1e-4
    }

    /// Kept inside the frame and at least 2% wide and high.
    pub fn clamped(&self) -> Crop {
        let width = self.width.clamp(0.02, 1.0);
        let height = self.height.clamp(0.02, 1.0);
        Crop {
            x: self.x.clamp(0.0, 1.0 - width),
            y: self.y.clamp(0.0, 1.0 - height),
            width,
            height,
            aspect: self.aspect.clone(),
        }
    }
}

/// Sensor noise reduction strengths, 0–100. Like Camera Raw, colour noise reduction
/// is on by default for RAW files and luminance smoothing is off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NoiseReduction {
    pub luminance: f32,
    pub color: f32,
}

impl Default for NoiseReduction {
    fn default() -> Self {
        Self {
            luminance: 0.0,
            color: 25.0,
        }
    }
}

/// PRD Step 4. Clarity and micro-texture run −100…100 (negative softens); the skin
/// retouching controls run 0…100 and only act where skin is detected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Texture {
    /// Mid-scale local contrast in the midtones.
    pub clarity: f32,
    /// Fine structure (hair, fabric, bark), cored against noise and eased off on skin.
    pub micro_texture: f32,
    /// Evens out spot-sized blemishes on skin while keeping pores and facial lines.
    pub blemish_smoothing: f32,
    /// Tames shiny hot spots on skin and restores the skin colour under them.
    pub specular_balance: f32,
    /// Character line sculpting, −100…100: deepens (+) or softens (−) the mid-scale
    /// lines of a face (smile lines, brow furrows) without touching pores.
    pub character_lines: f32,
    /// Confine skin retouching and line sculpting to the subject mask, so skin-toned
    /// backgrounds (wood, sand, brick) are left alone.
    pub retouch_subject_only: bool,
    /// 0…100: whitens teeth through the Teeth mask (less yellow, a touch brighter),
    /// leaving the lips alone.
    pub teeth_whitening: f32,
}

impl Texture {
    pub fn is_neutral(&self) -> bool {
        self.clarity == 0.0
            && self.micro_texture == 0.0
            && self.blemish_smoothing == 0.0
            && self.specular_balance == 0.0
            && self.character_lines == 0.0
    }
}

/// One HSL band: each value −100…100.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct HslChannel {
    pub hue: f32,
    pub saturation: f32,
    pub luminance: f32,
}

/// PRD Step 5 HSL: eight hue bands, as in Lightroom's HSL panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct HslBands {
    pub red: HslChannel,
    pub orange: HslChannel,
    pub yellow: HslChannel,
    pub green: HslChannel,
    pub aqua: HslChannel,
    pub blue: HslChannel,
    pub purple: HslChannel,
    pub magenta: HslChannel,
}

impl HslBands {
    /// Band names in hue order, matching [`HslBands::bands`].
    pub const NAMES: [&'static str; 8] = [
        "red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta",
    ];

    pub fn bands(&self) -> [&HslChannel; 8] {
        [
            &self.red,
            &self.orange,
            &self.yellow,
            &self.green,
            &self.aqua,
            &self.blue,
            &self.purple,
            &self.magenta,
        ]
    }

    pub fn bands_mut(&mut self) -> [&mut HslChannel; 8] {
        [
            &mut self.red,
            &mut self.orange,
            &mut self.yellow,
            &mut self.green,
            &mut self.aqua,
            &mut self.blue,
            &mut self.purple,
            &mut self.magenta,
        ]
    }
}

/// One colour wheel: a tint (hue in degrees on the usual HSV wheel, amount 0…100)
/// and a luminance offset (−100…100) for one tonal range.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ColorWheel {
    pub hue: f32,
    pub amount: f32,
    pub luminance: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ColorWheels {
    pub shadows: ColorWheel,
    pub midtones: ColorWheel,
    pub highlights: ColorWheel,
}

/// PRD Step 5: base colour grading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ColorGrade {
    pub hsl: HslBands,
    pub wheels: ColorWheels,
    /// 0…100: how much of the HSL and wheel changes skin is shielded from.
    pub skin_protection: f32,
    /// Foliage shift: hue (− towards teal, + towards autumn gold), saturation and
    /// luminance of greenery, outside the subject.
    pub foliage: HslChannel,
    /// Background re-colouration through the Step 3 subject mask.
    pub background: BackgroundTint,
}

/// Tint, saturation and luminance of everything outside the subject. Hue in degrees
/// on the HSV wheel, amount 0…100, saturation and luminance −100…100.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BackgroundTint {
    pub hue: f32,
    pub amount: f32,
    pub saturation: f32,
    pub luminance: f32,
}

impl BackgroundTint {
    pub fn is_neutral(&self) -> bool {
        self.amount <= 0.0 && self.saturation == 0.0 && self.luminance == 0.0
    }
}

/// PRD Step 6: atmospheric light. Strengths 0…100, warmth −100 (cool) … 100 (gold).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Atmosphere {
    /// Bloom around bright areas.
    pub glow: f32,
    /// Bloom radius, 0…100 (≈0.5–5% of the frame).
    pub glow_size: f32,
    pub glow_warmth: f32,
    /// Let only the subject glow (through the Step 3 subject mask).
    pub glow_subject_only: bool,
    /// Aerial haze that thickens with distance (from the depth model; uniform without it).
    pub fog: f32,
    /// Depth at which fog begins, 0 (at the camera) … 100 (only the far background).
    pub fog_start: f32,
    pub fog_warmth: f32,
    /// Volumetric rays streaming from a light source through the bright areas.
    pub shafts: f32,
    /// Ray length, 0…100 (fraction of the way to the light).
    pub shaft_length: f32,
    pub shaft_warmth: f32,
    /// Find the light source automatically (the brightest part of the sky).
    pub shaft_auto: bool,
    /// Light position when not automatic, 0–1 across and down the upright frame.
    pub shaft_x: f32,
    pub shaft_y: f32,
    /// 3D Atmospheric Light Sculptor (PRD Section 4): virtual lights placed in the
    /// scene's depth.
    pub lights: Vec<VirtualLight>,
}

/// A light placed in 3D after capture. `x`, `y` are 0–1 across and down the upright
/// frame; `depth` is 0 (at the camera) … 1 (the far background), on the same scale as
/// the depth map, so a light "behind" a subject is one with a greater depth than it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VirtualLight {
    pub x: f32,
    pub y: f32,
    pub depth: f32,
    /// 0…100.
    pub intensity: f32,
    /// How far the light reaches, 0…100.
    pub reach: f32,
    /// −100 (cool daylight) … 100 (warm tungsten / low sun).
    pub warmth: f32,
    /// Visible glow of the light in the air, 0…100.
    pub halo: f32,
}

impl Default for VirtualLight {
    fn default() -> Self {
        Self {
            x: 0.5,
            y: 0.4,
            depth: 0.5,
            intensity: 50.0,
            reach: 50.0,
            warmth: 40.0,
            halo: 40.0,
        }
    }
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self {
            glow: 0.0,
            glow_size: 50.0,
            glow_warmth: 0.0,
            glow_subject_only: false,
            fog: 0.0,
            fog_start: 30.0,
            fog_warmth: 0.0,
            shafts: 0.0,
            shaft_length: 60.0,
            shaft_warmth: 40.0,
            shaft_auto: true,
            shaft_x: 0.5,
            shaft_y: 0.15,
            lights: Vec::new(),
        }
    }
}

/// One parametric tone curve (PRD Step 7). Every value −100…100; all zero is the
/// identity. The four regions bend the curve around ⅕, ⅖, ⅗ and ⅘ of the tonal range;
/// `black` lifts the floor (matte, > 0) or crushes shadows to black (< 0); `white`
/// clips highlights earlier (> 0) or lowers the ceiling (faded, < 0).
///
/// `points` is a free-form point curve applied after the parametric one: control
/// points `[input, output]` in 0…1, sorted by input, joined by a monotone cubic.
/// Empty (or points on the diagonal) is the identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ToneCurve {
    pub shadows: f32,
    pub darks: f32,
    pub lights: f32,
    pub highlights: f32,
    pub black: f32,
    pub white: f32,
    pub points: Vec<[f32; 2]>,
}

impl ToneCurve {
    pub fn is_identity(&self) -> bool {
        self.to_array() == [0.0; 6] && self.points_are_identity()
    }

    /// No point curve, or one whose points all lie on the diagonal.
    pub fn points_are_identity(&self) -> bool {
        self.points.iter().all(|[x, y]| (x - y).abs() < 1e-6)
    }

    /// `[shadows, darks, lights, highlights, black, white]`.
    pub fn to_array(&self) -> [f32; 6] {
        [
            self.shadows,
            self.darks,
            self.lights,
            self.highlights,
            self.black,
            self.white,
        ]
    }

    pub fn from_array([shadows, darks, lights, highlights, black, white]: [f32; 6]) -> Self {
        Self {
            shadows,
            darks,
            lights,
            highlights,
            black,
            white,
            points: Vec::new(),
        }
    }
}

/// The master RGB curve (applied first) and one curve per channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Curves {
    pub rgb: ToneCurve,
    pub red: ToneCurve,
    pub green: ToneCurve,
    pub blue: ToneCurve,
    /// A standard contrast S-curve under the master curve.
    pub s_curve: SCurve,
}

impl Curves {
    pub fn is_identity(&self) -> bool {
        !self.s_curve.is_active() && [&self.rgb, &self.red, &self.green, &self.blue].iter().all(|c| c.is_identity())
    }
}

/// PRD Step 7 "Standard S-Curve": darkens below the pivot and brightens above it, black
/// and white fixed. `amount` 0…100; `pivot` 0…100 is the tone it turns around (50 =
/// the middle of the tonal range).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SCurve {
    pub enabled: bool,
    pub amount: f32,
    pub pivot: f32,
}

impl Default for SCurve {
    fn default() -> Self {
        Self { enabled: false, amount: 50.0, pivot: 50.0 }
    }
}

impl SCurve {
    pub fn is_active(&self) -> bool {
        self.enabled && self.amount > 0.0
    }
}

/// PRD Step 7 split toning: hue in degrees on the HSV wheel, saturation 0…100,
/// balance −100 (favour shadows) … 100 (favour highlights).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SplitToning {
    pub highlight_hue: f32,
    pub highlight_saturation: f32,
    pub shadow_hue: f32,
    pub shadow_saturation: f32,
    pub balance: f32,
}

impl Default for SplitToning {
    fn default() -> Self {
        // Hues start on the classic warm highlights / cool shadows pairing.
        Self {
            highlight_hue: 40.0,
            highlight_saturation: 0.0,
            shadow_hue: 215.0,
            shadow_saturation: 0.0,
            balance: 0.0,
        }
    }
}

impl SplitToning {
    pub fn is_neutral(&self) -> bool {
        self.highlight_saturation <= 0.0 && self.shadow_saturation <= 0.0
    }
}

/// PRD Step 8 finishing. Grain 0…100 with size and roughness 0…100; vignette amount
/// −100 (darken) … 100 (lighten), midpoint and feather 0…100, roundness −100 (follows
/// the frame) … 100 (circle).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Finishing {
    pub grain: f32,
    pub grain_size: f32,
    pub grain_roughness: f32,
    pub vignette: f32,
    pub vignette_midpoint: f32,
    pub vignette_feather: f32,
    pub vignette_roundness: f32,
}

impl Default for Finishing {
    fn default() -> Self {
        Self {
            grain: 0.0,
            grain_size: 25.0,
            grain_roughness: 50.0,
            vignette: 0.0,
            vignette_midpoint: 50.0,
            vignette_feather: 50.0,
            vignette_roundness: 0.0,
        }
    }
}

impl Finishing {
    pub fn is_neutral(&self) -> bool {
        self.grain <= 0.0 && self.vignette == 0.0
    }
}

/// A parametric style from the style engine, applied on top of the manual settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StyleRef {
    /// Style id (e.g. `"dark-melanin-glow"`); empty for none.
    pub id: String,
    /// 0…100: scales every parameter of the style.
    pub amount: f32,
    /// 0…100: how much of the style's scene grade skin is shielded from. Seeded from
    /// the style's own default when it is applied.
    pub skin_protection: f32,
    /// AI Style Fusion Matrix (PRD Section 4): up to four styles blended by weight.
    /// When set it replaces `id`; `amount` still scales the whole blend.
    pub blend: Vec<StyleWeight>,
}

/// One style's share of a fusion blend; weights are normalised when applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct StyleWeight {
    pub id: String,
    pub weight: f32,
}

impl Default for StyleRef {
    fn default() -> Self {
        Self {
            id: String::new(),
            amount: 100.0,
            skin_protection: 0.0,
            blend: Vec::new(),
        }
    }
}

impl StyleRef {
    pub fn is_none(&self) -> bool {
        (self.id.is_empty() && self.blend.iter().all(|b| b.weight <= 0.0)) || self.amount <= 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DemosaicMode {
    #[default]
    Auto,
    Malvar,
    Bilinear,
    Xtrans,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhiteBalance {
    pub mode: WbMode,
    pub temperature: f32,
    pub tint: f32,
}

impl Default for WhiteBalance {
    fn default() -> Self {
        Self {
            mode: WbMode::AsShot,
            temperature: 6500.0,
            tint: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WbMode {
    #[default]
    AsShot,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LensCorrections {
    /// Apply the lens profile: the camera's own corrections in a DNG, else the Lensfun
    /// database entry for the lens (distortion, lateral CA, vignetting).
    pub profile: bool,
    /// Manual corrections, applied on top of the profile.
    pub distortion: DistortionCoeffs,
    pub chromatic_aberration: ChromaticAberration,
    /// Straighten: rotation in degrees (positive = counter-clockwise), with the frame
    /// cropped to stay filled.
    pub rotation: f32,
    /// Vertical perspective, −100…100: positive straightens verticals that converge
    /// towards the top (camera tilted up).
    pub vertical: f32,
}

impl Default for LensCorrections {
    fn default() -> Self {
        Self {
            profile: true,
            distortion: DistortionCoeffs::default(),
            chromatic_aberration: ChromaticAberration::default(),
            rotation: 0.0,
            vertical: 0.0,
        }
    }
}

impl LensCorrections {
    /// Whether the upright geometry (rotation, perspective) changes anything.
    pub fn has_transform(&self) -> bool {
        self.rotation != 0.0 || self.vertical != 0.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistortionCoeffs {
    pub enabled: bool,
    pub k1: f32,
    pub k2: f32,
    pub k3: f32,
    pub p1: f32,
    pub p2: f32,
    pub cx: f32,
    pub cy: f32,
}

impl Default for DistortionCoeffs {
    fn default() -> Self {
        Self {
            enabled: false,
            k1: 0.0,
            k2: 0.0,
            k3: 0.0,
            p1: 0.0,
            p2: 0.0,
            cx: 0.0,
            cy: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromaticAberration {
    pub enabled: bool,
    /// Radial scale offset for red vs green (typical ±0.02).
    pub red: f32,
    /// Radial scale offset for blue vs green.
    pub blue: f32,
}

impl Default for ChromaticAberration {
    fn default() -> Self {
        Self {
            enabled: false,
            red: 0.0,
            blue: 0.0,
        }
    }
}
