// Dev-only stand-in for the Rust engine so the UI can be exercised in a plain browser
// (`npm run dev`, then open http://localhost:5173/?mock). It renders a synthetic test
// chart and applies exposure / white balance approximately. Never shipped: main.tsx only
// imports this behind `import.meta.env.DEV`.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type {
  Adjustments,
  DevelopDocument,
  FileEntry,
  ImageInfo,
  LocalAdjustment,
  MaskTarget,
  SceneAnalysis,
  StoryArc,
  StyleInfo,
  SyncReport,
} from "../types";
import { defaultAdjustments, defaultLocal } from "../types";

const FOLDER = "/mock/Sydney shoot";
const FILES: FileEntry[] = [
  "DSCF0346.RAF",
  "DSCF0352.RAF",
  "L1000530.DNG",
  "L1000583.DNG",
  "IMG_0421.CR3",
  "IMG_0433.JPG",
].map((name) => ({
  path: `${FOLDER}/${name}`,
  name,
  format: name.endsWith("RAF")
    ? "Fujifilm RAF"
    : name.endsWith("DNG")
      ? "Leica DNG"
      : name.endsWith("JPG")
        ? "JPEG"
        : "Canon CR3",
  hasEdits: false,
}));
const saved = new Map<string, DevelopDocument>();
// ~/.epikos stand-ins for the browser mock (kept in localStorage like the real files persist).
const LEARNED_KEY = "epikos.mock.learned";
const PRESETS_KEY = "epikos.mock.presets";
const load = <T,>(k: string): T[] => {
  try {
    return JSON.parse(localStorage.getItem(k) ?? "[]") as T[];
  } catch {
    return [];
  }
};
const store = (k: string, v: unknown) => {
  try {
    localStorage.setItem(k, JSON.stringify(v));
  } catch {
    // Mock only.
  }
};
const presync = new Map<string, DevelopDocument | null>();
const luts: { name: string; title: string | null; size: number }[] = [];
const AS_SHOT = { temperature: 5600, tint: 4 };

// Generated with `epikos styles` (crates/epikos-pipeline/src/look/style.rs).
const STYLES: StyleInfo[] = [
  {
    "id": "portra-400",
    "name": "Portra 400",
    "world": "Cinematic & Film Emulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Kodak's portrait stock: warm, forgiving skin, soft contrast, gently faded blacks and pastel greens and blues.",
    "skinProtection": 40.0,
    "swatch": [
      "#b99a7e",
      "#f0d5b8"
    ]
  },
  {
    "id": "fuji-pro-400h",
    "name": "Fuji Pro 400H",
    "world": "Cinematic & Film Emulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Fujifilm's wedding stock: airy pastels, minty greens, cool-green shadows and soft, bright highlights.",
    "skinProtection": 45.0,
    "swatch": [
      "#8fb3a3",
      "#eef2e6"
    ]
  },
  {
    "id": "cinestill-800t",
    "name": "CineStill 800T",
    "world": "Cinematic & Film Emulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Tungsten motion-picture stock: cool blue night colour, a red-orange halation glow around lights and visible grain.",
    "skinProtection": 50.0,
    "swatch": [
      "#1d2b4a",
      "#e0503a"
    ]
  },
  {
    "id": "leica-monochrom",
    "name": "Leica Monochrom",
    "world": "Character & High-Contrast Portraiture",
    "category": "Film Simulations",
    "listed": true,
    "description": "Black and white with the smooth, deep tonality of a monochrome sensor: rich midtones, fine texture, skin lifted from dark clothing.",
    "skinProtection": 60.0,
    "swatch": [
      "#1a1a1a",
      "#e8e8e8"
    ]
  },
  {
    "id": "ilford-hp5",
    "name": "Ilford HP5",
    "world": "Character & High-Contrast Portraiture",
    "category": "Film Simulations",
    "listed": true,
    "description": "Classic black-and-white press film: punchy contrast, dark skies and a gritty, visible grain.",
    "skinProtection": 55.0,
    "swatch": [
      "#0e0e0e",
      "#cfcfcf"
    ]
  },
  {
    "id": "fuji-provia",
    "name": "Provia / Standard",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Fujifilm's all-rounder: faithful colour, moderate contrast and saturation.",
    "skinProtection": 40.0,
    "swatch": [
      "#5b7fb0",
      "#e0c9a0"
    ]
  },
  {
    "id": "fuji-velvia",
    "name": "Velvia / Vivid",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Slide-film punch: deep blues, rich greens and reds, high contrast. Best for landscapes.",
    "skinProtection": 60.0,
    "swatch": [
      "#1f4fa8",
      "#2e8b3a"
    ]
  },
  {
    "id": "fuji-astia",
    "name": "Astia / Soft",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Soft contrast for portraits with bright, clean blues and greens kept lively.",
    "skinProtection": 55.0,
    "swatch": [
      "#e8c4a8",
      "#8cc0d8"
    ]
  },
  {
    "id": "fuji-classic-chrome",
    "name": "Classic Chrome",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Documentary colour: muted, cyan-leaning blues, subdued reds, firm shadows, a quiet brownish warmth.",
    "skinProtection": 45.0,
    "swatch": [
      "#5e7a80",
      "#b59a7a"
    ]
  },
  {
    "id": "fuji-reala-ace",
    "name": "Reala Ace",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "True-to-life colour with crisp, slightly hard tonality: an everyday negative look.",
    "skinProtection": 40.0,
    "swatch": [
      "#6f8fb3",
      "#d9b48f"
    ]
  },
  {
    "id": "fuji-pro-neg-hi",
    "name": "Pro Neg. Hi",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Portrait negative with a little extra contrast: natural skin, gently saturated backgrounds.",
    "skinProtection": 60.0,
    "swatch": [
      "#d8b294",
      "#6f8a9c"
    ]
  },
  {
    "id": "fuji-pro-neg-std",
    "name": "Pro Neg. Std",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Flat, soft studio negative: smooth skin gradation and muted colour for later grading.",
    "skinProtection": 60.0,
    "swatch": [
      "#d9bca6",
      "#9aa7ab"
    ]
  },
  {
    "id": "fuji-classic-neg",
    "name": "Classic Neg.",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Everyday consumer-negative feel: hard contrast, low saturation, cyan-green shadows and warm highlights.",
    "skinProtection": 40.0,
    "swatch": [
      "#4f7a72",
      "#d8a878"
    ]
  },
  {
    "id": "fuji-nostalgic-neg",
    "name": "Nostalgic Neg.",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "American New Colour warmth: amber highlights, softened brights, rich but gentle shadows.",
    "skinProtection": 40.0,
    "swatch": [
      "#c08850",
      "#6a5a4a"
    ]
  },
  {
    "id": "fuji-eterna",
    "name": "Eterna / Cinema",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Motion-picture stock: very soft contrast, restrained saturation and teal-leaning shadows; made for grading.",
    "skinProtection": 45.0,
    "swatch": [
      "#3e5a5c",
      "#b8a88a"
    ]
  },
  {
    "id": "fuji-eterna-bleach",
    "name": "Eterna Bleach Bypass",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "The skipped-bleach film process: very low saturation, hard contrast, silvery highlights.",
    "skinProtection": 30.0,
    "swatch": [
      "#50575a",
      "#c9c6bd"
    ]
  },
  {
    "id": "fuji-acros",
    "name": "Acros",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Fine-grained black and white with deep blacks, smooth midtones and crisp detail.",
    "skinProtection": 20.0,
    "swatch": [
      "#111111",
      "#dddddd"
    ]
  },
  {
    "id": "fuji-acros-ye",
    "name": "Acros + Ye filter",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Acros through a yellow filter: skies a touch darker, clouds and skin slightly brighter.",
    "skinProtection": 20.0,
    "swatch": [
      "#151515",
      "#e0dccb"
    ]
  },
  {
    "id": "fuji-acros-r",
    "name": "Acros + R filter",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Acros through a red filter: dramatic dark skies, luminous skin and brick, bold contrast.",
    "skinProtection": 20.0,
    "swatch": [
      "#0c0c0c",
      "#eee4dc"
    ]
  },
  {
    "id": "fuji-acros-g",
    "name": "Acros + G filter",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Acros through a green filter: bright foliage, deeper lips and skin tones for character portraits.",
    "skinProtection": 20.0,
    "swatch": [
      "#141414",
      "#dfe6dc"
    ]
  },
  {
    "id": "fuji-monochrome",
    "name": "Monochrome",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Straight black and white: neutral mix, standard contrast, no grain.",
    "skinProtection": 20.0,
    "swatch": [
      "#262626",
      "#d4d4d4"
    ]
  },
  {
    "id": "fuji-monochrome-ye",
    "name": "Monochrome + Ye filter",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Standard black and white through a yellow filter.",
    "skinProtection": 20.0,
    "swatch": [
      "#282828",
      "#dcd8c8"
    ]
  },
  {
    "id": "fuji-monochrome-r",
    "name": "Monochrome + R filter",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Standard black and white through a red filter: darker skies, lighter skin.",
    "skinProtection": 20.0,
    "swatch": [
      "#202020",
      "#e6ddd4"
    ]
  },
  {
    "id": "fuji-monochrome-g",
    "name": "Monochrome + G filter",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Standard black and white through a green filter: lighter foliage, richer skin.",
    "skinProtection": 20.0,
    "swatch": [
      "#222222",
      "#dbe2d8"
    ]
  },
  {
    "id": "fuji-sepia",
    "name": "Sepia",
    "world": "Fujifilm-inspired Film Simulation",
    "category": "Film Simulations",
    "listed": true,
    "description": "Warm brown-toned monochrome, like an old print.",
    "skinProtection": 20.0,
    "swatch": [
      "#4a3624",
      "#e2cfb0"
    ]
  },
  {
    "id": "leica-standard",
    "name": "Leica Standard",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "Leica's natural rendering: accurate colour, gentle contrast, clean highlights.",
    "skinProtection": 40.0,
    "swatch": [
      "#6c7f95",
      "#d8c1a6"
    ]
  },
  {
    "id": "leica-vivid",
    "name": "Leica Vivid",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "More saturated and contrasty, with bright, clear colour.",
    "skinProtection": 50.0,
    "swatch": [
      "#2d62b8",
      "#e3a33b"
    ]
  },
  {
    "id": "leica-natural",
    "name": "Leica Natural",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "Lower contrast and saturation: soft, true-to-life files for later grading.",
    "skinProtection": 50.0,
    "swatch": [
      "#8395a3",
      "#d9c8b4"
    ]
  },
  {
    "id": "leica-classic",
    "name": "Leica Classic",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "Analogue-era colour: warm, slightly muted, gentle contrast and soft highlights.",
    "skinProtection": 40.0,
    "swatch": [
      "#9c7a55",
      "#d6c4a2"
    ]
  },
  {
    "id": "leica-contemporary",
    "name": "Leica Contemporary",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "Modern and cool: clean whites, cyan-leaning shadows, restrained saturation.",
    "skinProtection": 40.0,
    "swatch": [
      "#50707e",
      "#e6e8e8"
    ]
  },
  {
    "id": "leica-eternal",
    "name": "Leica Eternal",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "Cinematic: soft, muted, teal shadows and warm skin, like a motion-picture stock.",
    "skinProtection": 50.0,
    "swatch": [
      "#35565a",
      "#c9a27c"
    ]
  },
  {
    "id": "leica-chrome",
    "name": "Leica Chrome",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "Slide-film colour: saturated, deep blues and greens, fairly firm contrast.",
    "skinProtection": 45.0,
    "swatch": [
      "#234f86",
      "#c7843c"
    ]
  },
  {
    "id": "leica-chrome-hc",
    "name": "Leica Chrome High Contrast",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "Leica Chrome with hard contrast and deep shadows: bold street and travel colour.",
    "skinProtection": 45.0,
    "swatch": [
      "#162f5a",
      "#d68a38"
    ]
  },
  {
    "id": "leica-sepia",
    "name": "Leica Sepia",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "Warm-toned monochrome with soft, rounded highlights.",
    "skinProtection": 20.0,
    "swatch": [
      "#503a26",
      "#e5d2b3"
    ]
  },
  {
    "id": "leica-selenium",
    "name": "Leica Selenium",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "Selenium-toned print: cool purple-brown shadows, neutral highlights, deep blacks.",
    "skinProtection": 20.0,
    "swatch": [
      "#2f2733",
      "#dedad6"
    ]
  },
  {
    "id": "leica-blue",
    "name": "Leica Blue",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "Cyanotype-like blue-toned monochrome.",
    "skinProtection": 20.0,
    "swatch": [
      "#1e3550",
      "#d4e0ea"
    ]
  },
  {
    "id": "leica-bw-natural",
    "name": "Leica B&W Natural",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "Black and white with natural contrast and a long, smooth grey scale.",
    "skinProtection": 20.0,
    "swatch": [
      "#2a2a2a",
      "#d8d8d8"
    ]
  },
  {
    "id": "leica-bw-hc",
    "name": "Leica B&W High Contrast",
    "world": "Leica-inspired Look",
    "category": "Film Simulations",
    "listed": true,
    "description": "Punchy black and white: dense blacks, bright whites, a darker sky.",
    "skinProtection": 20.0,
    "swatch": [
      "#060606",
      "#f2f2f2"
    ]
  },
  {
    "id": "dark-melanin-glow",
    "name": "Dark Melanin Glow",
    "world": "High-Fashion & Melanin Precision",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Rich chocolate and bronze undertones with a clean, luminous glow. Shine is balanced, not flattened; whites stay neutral.",
    "skinProtection": 70.0,
    "swatch": [
      "#3b2116",
      "#c98a4b"
    ]
  },
  {
    "id": "soft-editorial",
    "name": "Soft Editorial",
    "world": "High-Fashion & Melanin Precision",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Magazine-clean and gentle: soft skin, lifted shadows, quiet background colour and low contrast.",
    "skinProtection": 70.0,
    "swatch": [
      "#c7b6ae",
      "#f6ede8"
    ]
  },
  {
    "id": "amber-warmth",
    "name": "Amber Warmth",
    "world": "High-Fashion & Melanin Precision",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Golden-amber light: warm, glowing skin and honeyed highlights against cooled-down greens and blues.",
    "skinProtection": 55.0,
    "swatch": [
      "#6b3a18",
      "#f2a54a"
    ]
  },
  {
    "id": "high-key-beauty",
    "name": "High-Key Beauty",
    "world": "High-Fashion & Melanin Precision",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Bright beauty light: luminous, evenly smoothed skin, pastel shadows and gentle contrast.",
    "skinProtection": 60.0,
    "swatch": [
      "#e3d6dc",
      "#fbf5f1"
    ]
  },
  {
    "id": "porcelain-glow",
    "name": "Porcelain Glow",
    "world": "Beauty & Editorial Portraiture",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Luminous, even skin with a soft bloom and clean, slightly cool whites.",
    "skinProtection": 70.0,
    "swatch": [
      "#f2dfd4",
      "#c9d6e0"
    ]
  },
  {
    "id": "golden-skin",
    "name": "Golden Skin",
    "world": "Beauty & Editorial Portraiture",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Sun-kissed warmth on skin, honeyed highlights and softly warm shadows.",
    "skinProtection": 30.0,
    "swatch": [
      "#d49a5e",
      "#f3d9a8"
    ]
  },
  {
    "id": "clean-commercial",
    "name": "Clean Commercial",
    "world": "Beauty & Editorial Portraiture",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Bright, crisp and neutral: true skin, clean whites, a touch of clarity for headshots.",
    "skinProtection": 70.0,
    "swatch": [
      "#e9e4de",
      "#b3a08a"
    ]
  },
  {
    "id": "matte-portrait",
    "name": "Matte Portrait",
    "world": "Beauty & Editorial Portraiture",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Lifted, milky blacks and gentle colour: the soft matte look of modern editorial.",
    "skinProtection": 60.0,
    "swatch": [
      "#6e6862",
      "#d8cfc4"
    ]
  },
  {
    "id": "bronze-editorial",
    "name": "Bronze Editorial",
    "world": "Character & High-Contrast Portraiture",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Rich bronze skin, deep warm shadows and sculpted contrast; flattering on darker skin.",
    "skinProtection": 30.0,
    "swatch": [
      "#6a3f22",
      "#c98f5c"
    ]
  },
  {
    "id": "rosy-fresh",
    "name": "Rosy Fresh",
    "world": "Beauty & Editorial Portraiture",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Healthy pink-peach complexion, fresh greens and soft, bright light.",
    "skinProtection": 40.0,
    "swatch": [
      "#f0b4a8",
      "#bfe0c0"
    ]
  },
  {
    "id": "low-key-drama",
    "name": "Low-Key Drama",
    "world": "Character & High-Contrast Portraiture",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Dark, moody portrait: deep shadows, sculpted light on the face, muted surroundings.",
    "skinProtection": 60.0,
    "swatch": [
      "#141210",
      "#a0826a"
    ]
  },
  {
    "id": "window-light",
    "name": "Window Light",
    "world": "Beauty & Editorial Portraiture",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Natural daylight portraits: soft, slightly cool fill, honest skin and gentle contrast.",
    "skinProtection": 60.0,
    "swatch": [
      "#c7ced4",
      "#e0c4ac"
    ]
  },
  {
    "id": "wedding-airy",
    "name": "Wedding Airy",
    "world": "Beauty & Editorial Portraiture",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Light and airy: bright pastels, creamy whites, soft greens; the fine-art wedding look.",
    "skinProtection": 55.0,
    "swatch": [
      "#f4ede4",
      "#c8d8c0"
    ]
  },
  {
    "id": "classic-bw-portrait",
    "name": "Classic B&W Portrait",
    "world": "Character & High-Contrast Portraiture",
    "category": "Portraits & Skin",
    "listed": true,
    "description": "Timeless black-and-white portrait: glowing skin, rich blacks and fine texture.",
    "skinProtection": 20.0,
    "swatch": [
      "#161616",
      "#ede6e0"
    ]
  },
  {
    "id": "moody-pacific",
    "name": "Moody Pacific",
    "world": "Atmospheric & Environmental Landscapes",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Cold coast light: deep slate blues and teals, muted greens, dark shadows and a cool sea mist.",
    "skinProtection": 70.0,
    "swatch": [
      "#1f2f3a",
      "#8aa1ab"
    ]
  },
  {
    "id": "golden-hour-flare",
    "name": "Golden Hour Flare",
    "world": "Atmospheric & Environmental Landscapes",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Low sun straight into the lens: warm light, a strong golden bloom and rays through the bright sky.",
    "skinProtection": 75.0,
    "swatch": [
      "#5a3a1c",
      "#ffc15e"
    ]
  },
  {
    "id": "deep-emerald",
    "name": "Deep Emerald",
    "world": "Atmospheric & Environmental Landscapes",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Rich, dark forest greens and teals with deep shadows and crisp leaf texture.",
    "skinProtection": 70.0,
    "swatch": [
      "#0d2a1c",
      "#3f8a5a"
    ]
  },
  {
    "id": "vivid-alpine",
    "name": "Vivid Alpine",
    "world": "Atmospheric & Environmental Landscapes",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Crisp mountain air: deep blue skies, clear greens, bright snow and strong detail.",
    "skinProtection": 75.0,
    "swatch": [
      "#1a5fa8",
      "#e8f4ff"
    ]
  },
  {
    "id": "nordic-blue-hour",
    "name": "Nordic Blue Hour",
    "world": "Nature & Landscape",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Cold, calm twilight blues with a whisper of warm light at the horizon.",
    "skinProtection": 60.0,
    "swatch": [
      "#1f3350",
      "#e6b88a"
    ]
  },
  {
    "id": "autumn-gold",
    "name": "Autumn Gold",
    "world": "Nature & Landscape",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Fiery oranges, golden yellows and warm light for autumn foliage.",
    "skinProtection": 60.0,
    "swatch": [
      "#c0601c",
      "#e8b43a"
    ]
  },
  {
    "id": "desert-heat",
    "name": "Desert Heat",
    "world": "Nature & Landscape",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Sun-baked warmth: terracotta sands, deep blue skies, a dry, hazy glow.",
    "skinProtection": 60.0,
    "swatch": [
      "#c77c47",
      "#2a5c9e"
    ]
  },
  {
    "id": "misty-forest",
    "name": "Misty Forest",
    "world": "Nature & Landscape",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Muted mossy greens, soft fog and quiet contrast for woodland scenes.",
    "skinProtection": 60.0,
    "swatch": [
      "#46594a",
      "#c3cbc0"
    ]
  },
  {
    "id": "crisp-winter",
    "name": "Crisp Winter",
    "world": "Nature & Landscape",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Clean white snow, cold blue shadows and crisp detail.",
    "skinProtection": 60.0,
    "swatch": [
      "#dfe9f2",
      "#4f78a8"
    ]
  },
  {
    "id": "tropical-lagoon",
    "name": "Tropical Lagoon",
    "world": "Nature & Landscape",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Turquoise water, lush greens and bright sunny whites.",
    "skinProtection": 60.0,
    "swatch": [
      "#19b0b8",
      "#5aa845"
    ]
  },
  {
    "id": "mountain-drama",
    "name": "Mountain Drama",
    "world": "Nature & Landscape",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Deep skies, strong local contrast and textured rock for epic mountain scenes.",
    "skinProtection": 60.0,
    "swatch": [
      "#253444",
      "#b9b2a4"
    ]
  },
  {
    "id": "coastal-pastel",
    "name": "Coastal Pastel",
    "world": "Nature & Landscape",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Soft seaside pastels: pale aquas, sandy creams and lifted shadows.",
    "skinProtection": 60.0,
    "swatch": [
      "#a8d4d4",
      "#efe0c8"
    ]
  },
  {
    "id": "wildflower-meadow",
    "name": "Wildflower Meadow",
    "world": "Nature & Landscape",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Vivid blooms in soft, warm light: rich magentas and yellows over gentle greens.",
    "skinProtection": 60.0,
    "swatch": [
      "#c14f9a",
      "#e8c83a"
    ]
  },
  {
    "id": "night-sky",
    "name": "Night Sky",
    "world": "Nature & Landscape",
    "category": "Landscape & Nature",
    "listed": true,
    "description": "Astro and night landscapes: neutral-blue sky, crisp stars, controlled noise and warm ground light.",
    "skinProtection": 60.0,
    "swatch": [
      "#0b1630",
      "#6c7fb0"
    ]
  },
  {
    "id": "aerial-clean",
    "name": "Drone Clean",
    "world": "Aerial & Drone",
    "category": "Aerial",
    "listed": true,
    "description": "Cuts the haze of altitude: clear, neutral colour and crisp detail from above.",
    "skinProtection": 60.0,
    "swatch": [
      "#4d7aa6",
      "#9ab06a"
    ]
  },
  {
    "id": "aerial-turquoise-coast",
    "name": "Turquoise Coast",
    "world": "Aerial & Drone",
    "category": "Aerial",
    "listed": true,
    "description": "Glowing shallows, white surf and golden sand seen from above.",
    "skinProtection": 60.0,
    "swatch": [
      "#12b5c4",
      "#e8d6a8"
    ]
  },
  {
    "id": "aerial-urban-grid",
    "name": "Urban Grid",
    "world": "Aerial & Drone",
    "category": "Aerial",
    "listed": true,
    "description": "City blocks and roads: cool, graphic, contrasty and slightly desaturated.",
    "skinProtection": 60.0,
    "swatch": [
      "#3d4a58",
      "#c4c8cc"
    ]
  },
  {
    "id": "aerial-golden",
    "name": "Golden Aerial",
    "world": "Aerial & Drone",
    "category": "Aerial",
    "listed": true,
    "description": "Low sun from above: long shadows, golden light raking across the land.",
    "skinProtection": 60.0,
    "swatch": [
      "#d08a36",
      "#4a3a2a"
    ]
  },
  {
    "id": "aerial-patchwork",
    "name": "Patchwork Fields",
    "world": "Aerial & Drone",
    "category": "Aerial",
    "listed": true,
    "description": "Farmland mosaics: separated greens and yellows, rich earth and firm texture.",
    "skinProtection": 60.0,
    "swatch": [
      "#6c9a3a",
      "#d8b44a"
    ]
  },
  {
    "id": "aerial-minimal",
    "name": "Aerial Minimal",
    "world": "Aerial & Drone",
    "category": "Aerial",
    "listed": true,
    "description": "Clean, soft and restrained for minimal compositions: pale tones, gentle colour.",
    "skinProtection": 60.0,
    "swatch": [
      "#d6dde0",
      "#8fa8b0"
    ]
  },
  {
    "id": "aerial-blue-hour-city",
    "name": "Blue Hour City",
    "world": "Aerial & Drone",
    "category": "Aerial",
    "listed": true,
    "description": "Deep blue dusk with warm city lights glowing below.",
    "skinProtection": 60.0,
    "swatch": [
      "#142a52",
      "#f0a040"
    ]
  },
  {
    "id": "aerial-glacier",
    "name": "Glacier Ice",
    "world": "Aerial & Drone",
    "category": "Aerial",
    "listed": true,
    "description": "Icy cyans and whites with crisp crevasse detail.",
    "skinProtection": 60.0,
    "swatch": [
      "#bfe4ee",
      "#3a6f8f"
    ]
  },
  {
    "id": "aerial-desert-patterns",
    "name": "Desert Patterns",
    "world": "Aerial & Drone",
    "category": "Aerial",
    "listed": true,
    "description": "Dunes and dry riverbeds: warm ochres, deep shadow lines and strong texture.",
    "skinProtection": 60.0,
    "swatch": [
      "#c8834a",
      "#5a3a22"
    ]
  },
  {
    "id": "aerial-forest-canopy",
    "name": "Forest Canopy",
    "world": "Aerial & Drone",
    "category": "Aerial",
    "listed": true,
    "description": "Dense treetops: separated greens, deep shadows between crowns.",
    "skinProtection": 60.0,
    "swatch": [
      "#1f4a2a",
      "#7fae52"
    ]
  },
  {
    "id": "aerial-moody-coast",
    "name": "Moody Coast",
    "world": "Aerial & Drone",
    "category": "Aerial",
    "listed": true,
    "description": "Stormy seas from above: dark teal water, muted land, heavy contrast.",
    "skinProtection": 60.0,
    "swatch": [
      "#1e3c42",
      "#8a8f86"
    ]
  },
  {
    "id": "aerial-bw",
    "name": "Aerial B&W",
    "world": "Aerial & Drone",
    "category": "Aerial",
    "listed": true,
    "description": "Graphic black and white from above: shape, line and shadow.",
    "skinProtection": 20.0,
    "swatch": [
      "#121212",
      "#e8e8e8"
    ]
  },
  {
    "id": "bright-airy",
    "name": "Bright & Airy",
    "world": "Lifestyle & Everyday",
    "category": "Lifestyle",
    "listed": true,
    "description": "Clean, light-filled and soft: bright whites and gentle pastel colour.",
    "skinProtection": 55.0,
    "swatch": [
      "#f5f1ea",
      "#cfdad8"
    ]
  },
  {
    "id": "warm-home",
    "name": "Warm Home",
    "world": "Lifestyle & Everyday",
    "category": "Lifestyle",
    "listed": true,
    "description": "Cosy interiors: warm wood, soft lamps and inviting, gentle contrast.",
    "skinProtection": 45.0,
    "swatch": [
      "#b07a4a",
      "#f0dcc0"
    ]
  },
  {
    "id": "cafe-film",
    "name": "Café Film",
    "world": "Lifestyle & Everyday",
    "category": "Lifestyle",
    "listed": true,
    "description": "Film-like everyday moments: warm, slightly faded, with fine grain.",
    "skinProtection": 45.0,
    "swatch": [
      "#8a6a4e",
      "#d8c4a4"
    ]
  },
  {
    "id": "sunday-morning",
    "name": "Sunday Morning",
    "world": "Lifestyle & Everyday",
    "category": "Lifestyle",
    "listed": true,
    "description": "Soft morning light, creamy highlights and relaxed, low contrast.",
    "skinProtection": 55.0,
    "swatch": [
      "#f2e4cc",
      "#b8b0a0"
    ]
  },
  {
    "id": "coastal-living",
    "name": "Coastal Living",
    "world": "Lifestyle & Everyday",
    "category": "Lifestyle",
    "listed": true,
    "description": "Breezy blues, sandy neutrals and clean daylight.",
    "skinProtection": 55.0,
    "swatch": [
      "#7fb2c8",
      "#e8dcc4"
    ]
  },
  {
    "id": "street-candid",
    "name": "Street Candid",
    "world": "Lifestyle & Everyday",
    "category": "Lifestyle",
    "listed": true,
    "description": "Gritty city colour: muted palette, punchy contrast and texture.",
    "skinProtection": 45.0,
    "swatch": [
      "#4a4e52",
      "#b89a70"
    ]
  },
  {
    "id": "summer-fade",
    "name": "Summer Fade",
    "world": "Lifestyle & Everyday",
    "category": "Lifestyle",
    "listed": true,
    "description": "Sun-bleached holiday colour: warm, faded, slightly overexposed.",
    "skinProtection": 45.0,
    "swatch": [
      "#e8c09a",
      "#8ec4d0"
    ]
  },
  {
    "id": "cozy-tungsten",
    "name": "Cozy Tungsten",
    "world": "Lifestyle & Everyday",
    "category": "Lifestyle",
    "listed": true,
    "description": "Evening indoors: amber lamplight, deep warm shadows and a soft glow.",
    "skinProtection": 40.0,
    "swatch": [
      "#6a3a18",
      "#f0a850"
    ]
  },
  {
    "id": "food-fresh",
    "name": "Food Fresh",
    "world": "Lifestyle & Everyday",
    "category": "Lifestyle",
    "listed": true,
    "description": "Appetising food: fresh greens, rich reds, clean whites and crisp texture.",
    "skinProtection": 20.0,
    "swatch": [
      "#c8402a",
      "#6aa83a"
    ]
  },
  {
    "id": "travel-journal",
    "name": "Travel Journal",
    "world": "Lifestyle & Everyday",
    "category": "Lifestyle",
    "listed": true,
    "description": "Vivid but natural travel colour with warm light and a hint of film.",
    "skinProtection": 45.0,
    "swatch": [
      "#d08a4a",
      "#3a7ab0"
    ]
  },
  {
    "id": "urban-pastel",
    "name": "Urban Pastel",
    "world": "Lifestyle & Everyday",
    "category": "Lifestyle",
    "listed": true,
    "description": "Candy-coloured city: soft pinks, mint and sky blue with lifted shadows.",
    "skinProtection": 50.0,
    "swatch": [
      "#f0b8c8",
      "#a8e0d0"
    ]
  },
  {
    "id": "soft-matte-life",
    "name": "Soft Matte",
    "world": "Lifestyle & Everyday",
    "category": "Lifestyle",
    "listed": true,
    "description": "Muted, matte everyday look: lifted blacks and quiet colour for a cohesive feed.",
    "skinProtection": 50.0,
    "swatch": [
      "#7a746c",
      "#d4ccc0"
    ]
  },
  {
    "id": "essential-natural",
    "name": "Natural",
    "world": "Essentials",
    "category": "Essentials",
    "listed": true,
    "description": "A gentle starting point: a little contrast and vibrance, nothing more.",
    "skinProtection": 60.0,
    "swatch": [
      "#8a9aa8",
      "#d4c0a8"
    ]
  },
  {
    "id": "essential-clean-pop",
    "name": "Clean Pop",
    "world": "Essentials",
    "category": "Essentials",
    "listed": true,
    "description": "Clear, lively colour with crisp midtones.",
    "skinProtection": 60.0,
    "swatch": [
      "#3a7ac8",
      "#f0c040"
    ]
  },
  {
    "id": "essential-punchy",
    "name": "Punchy",
    "world": "Essentials",
    "category": "Essentials",
    "listed": true,
    "description": "Bold contrast and saturation for colour that jumps off the screen.",
    "skinProtection": 60.0,
    "swatch": [
      "#c8202a",
      "#1a4ab0"
    ]
  },
  {
    "id": "essential-warm",
    "name": "Warm Up",
    "world": "Essentials",
    "category": "Essentials",
    "listed": true,
    "description": "A gentle golden warmth across the image.",
    "skinProtection": 50.0,
    "swatch": [
      "#e0a060",
      "#f4dcb8"
    ]
  },
  {
    "id": "essential-cool",
    "name": "Cool Down",
    "world": "Essentials",
    "category": "Essentials",
    "listed": true,
    "description": "A calm, cooler balance with clean blue shadows.",
    "skinProtection": 50.0,
    "swatch": [
      "#5a80b0",
      "#d8e4ee"
    ]
  },
  {
    "id": "essential-crisp",
    "name": "Crisp Detail",
    "world": "Essentials",
    "category": "Essentials",
    "listed": true,
    "description": "Sharper-looking texture and midtone clarity without harsh contrast.",
    "skinProtection": 70.0,
    "swatch": [
      "#505a64",
      "#c8ccd0"
    ]
  },
  {
    "id": "essential-high-contrast",
    "name": "High Contrast",
    "world": "Essentials",
    "category": "Essentials",
    "listed": true,
    "description": "Deeper blacks and brighter whites.",
    "skinProtection": 60.0,
    "swatch": [
      "#101010",
      "#f0f0f0"
    ]
  },
  {
    "id": "essential-low-contrast",
    "name": "Low Contrast",
    "world": "Essentials",
    "category": "Essentials",
    "listed": true,
    "description": "Softer, flatter tones that hold detail in shadows and highlights.",
    "skinProtection": 60.0,
    "swatch": [
      "#6a6a6a",
      "#b8b8b8"
    ]
  },
  {
    "id": "essential-vivid",
    "name": "Vivid Colour",
    "world": "Essentials",
    "category": "Essentials",
    "listed": true,
    "description": "Stronger colour everywhere, with skin kept natural.",
    "skinProtection": 80.0,
    "swatch": [
      "#e03a8a",
      "#2ab0a0"
    ]
  },
  {
    "id": "essential-muted",
    "name": "Muted Colour",
    "world": "Essentials",
    "category": "Essentials",
    "listed": true,
    "description": "Quieter, understated colour.",
    "skinProtection": 50.0,
    "swatch": [
      "#8a8a80",
      "#b0a898"
    ]
  },
  {
    "id": "essential-bw",
    "name": "B&W Classic",
    "world": "Essentials",
    "category": "Essentials",
    "listed": true,
    "description": "A classic black-and-white conversion with a yellow-filter sky.",
    "skinProtection": 20.0,
    "swatch": [
      "#1a1a1a",
      "#e4e4e4"
    ]
  },
  {
    "id": "essential-bw-soft",
    "name": "B&W Soft",
    "world": "Essentials",
    "category": "Essentials",
    "listed": true,
    "description": "Gentle, low-contrast black and white with open shadows.",
    "skinProtection": 20.0,
    "swatch": [
      "#4a4a4a",
      "#dcdcdc"
    ]
  },
  {
    "id": "macro-clean",
    "name": "Macro Clean",
    "world": "Macro & Close-up",
    "category": "Macro",
    "listed": true,
    "description": "True colour with fine detail brought forward and smooth backgrounds kept smooth.",
    "skinProtection": 60.0,
    "swatch": [
      "#6a9a4a",
      "#e0d0a0"
    ]
  },
  {
    "id": "macro-petal-soft",
    "name": "Petal Soft",
    "world": "Macro & Close-up",
    "category": "Macro",
    "listed": true,
    "description": "Dreamy flowers: soft glow, pastel colour and gentle contrast.",
    "skinProtection": 60.0,
    "swatch": [
      "#f0c0d8",
      "#e8f0d8"
    ]
  },
  {
    "id": "macro-dewdrop",
    "name": "Dewdrop Fresh",
    "world": "Macro & Close-up",
    "category": "Macro",
    "listed": true,
    "description": "Morning freshness: cool, sparkling highlights and vivid greens.",
    "skinProtection": 60.0,
    "swatch": [
      "#48a060",
      "#d8f0f4"
    ]
  },
  {
    "id": "macro-insect-detail",
    "name": "Insect Detail",
    "world": "Macro & Close-up",
    "category": "Macro",
    "listed": true,
    "description": "Maximum fine texture and micro-contrast for insects and small creatures.",
    "skinProtection": 70.0,
    "swatch": [
      "#3a4a2a",
      "#c8a040"
    ]
  },
  {
    "id": "macro-botanical-dark",
    "name": "Botanical Dark",
    "world": "Macro & Close-up",
    "category": "Macro",
    "listed": true,
    "description": "Dutch-still-life mood: deep, dark backgrounds and rich, glowing subjects.",
    "skinProtection": 60.0,
    "swatch": [
      "#101410",
      "#c05070"
    ]
  },
  {
    "id": "macro-pastel-bloom",
    "name": "Pastel Bloom",
    "world": "Macro & Close-up",
    "category": "Macro",
    "listed": true,
    "description": "Light, airy pastels with lifted shadows for spring flowers.",
    "skinProtection": 60.0,
    "swatch": [
      "#f4d0e0",
      "#d0e8f0"
    ]
  },
  {
    "id": "macro-leaf-glow",
    "name": "Leaf Glow",
    "world": "Macro & Close-up",
    "category": "Macro",
    "listed": true,
    "description": "Backlit leaves: luminous yellow-greens and a warm glow.",
    "skinProtection": 60.0,
    "swatch": [
      "#a8c830",
      "#f0d060"
    ]
  },
  {
    "id": "macro-texture-pop",
    "name": "Texture Pop",
    "world": "Macro & Close-up",
    "category": "Macro",
    "listed": true,
    "description": "Surfaces and patterns: strong clarity, micro-contrast and slightly muted colour.",
    "skinProtection": 70.0,
    "swatch": [
      "#5a4a3a",
      "#b8a890"
    ]
  },
  {
    "id": "macro-autumn-leaf",
    "name": "Autumn Leaf",
    "world": "Macro & Close-up",
    "category": "Macro",
    "listed": true,
    "description": "Close-up autumn colour: glowing reds and oranges, warm and rich.",
    "skinProtection": 60.0,
    "swatch": [
      "#c04418",
      "#e89a30"
    ]
  },
  {
    "id": "macro-water-blue",
    "name": "Water Drop Blue",
    "world": "Macro & Close-up",
    "category": "Macro",
    "listed": true,
    "description": "Cool blues and crystal-clear droplets.",
    "skinProtection": 60.0,
    "swatch": [
      "#2a6ab0",
      "#c8e4f4"
    ]
  },
  {
    "id": "macro-product",
    "name": "Studio Product",
    "world": "Macro & Close-up",
    "category": "Macro",
    "listed": true,
    "description": "Clean catalogue close-ups: neutral whites, accurate colour, crisp edges.",
    "skinProtection": 60.0,
    "swatch": [
      "#f4f4f4",
      "#9aa0a8"
    ]
  },
  {
    "id": "macro-mono",
    "name": "Mono Macro",
    "world": "Macro & Close-up",
    "category": "Macro",
    "listed": true,
    "description": "Black-and-white close-ups that celebrate form and texture.",
    "skinProtection": 20.0,
    "swatch": [
      "#141414",
      "#e0e0e0"
    ]
  },
  {
    "id": "teal-orange",
    "name": "Teal & Orange",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "Blockbuster grade: teal shadows and backgrounds against warm highlights. Skin stays warm and natural.",
    "skinProtection": 80.0,
    "swatch": [
      "#0f4c55",
      "#e08a3c"
    ]
  },
  {
    "id": "bleach-bypass",
    "name": "Bleach Bypass",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "The skipped-bleach film process: muted, silvery colour with hard contrast and gritty detail.",
    "skinProtection": 30.0,
    "swatch": [
      "#2b2e30",
      "#b8b4a8"
    ]
  },
  {
    "id": "cyberpunk",
    "name": "Cyberpunk",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "Neon night city: purple shadows, magenta and cyan light, glowing highlights.",
    "skinProtection": 45.0,
    "swatch": [
      "#2a0f4a",
      "#ff3cac"
    ]
  },
  {
    "id": "vintage-pastel",
    "name": "Vintage Pastel",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "A faded 1970s print: soft pastels, pink highlights, green-cyan shadows and gentle grain.",
    "skinProtection": 45.0,
    "swatch": [
      "#a8c9c2",
      "#f5d6d9"
    ]
  },
  {
    "id": "blockbuster",
    "name": "Blockbuster",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "Big-screen action grade: strong teal shadows, warm skin, heavy contrast.",
    "skinProtection": 45.0,
    "swatch": [
      "#0f4a52",
      "#e08a4a"
    ]
  },
  {
    "id": "neo-noir",
    "name": "Neo-Noir",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "Dark, desaturated crime-drama look with cold greens and hard shadows.",
    "skinProtection": 45.0,
    "swatch": [
      "#0e1612",
      "#6e8078"
    ]
  },
  {
    "id": "western-dust",
    "name": "Western Dust",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "Sun-scorched frontier: dusty ambers, faded blacks, grain.",
    "skinProtection": 40.0,
    "swatch": [
      "#a0703a",
      "#e0c89a"
    ]
  },
  {
    "id": "nordic-noir",
    "name": "Nordic Noir",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "Scandinavian drama: cold, grey-blue, muted and bleak.",
    "skinProtection": 45.0,
    "swatch": [
      "#34404a",
      "#a8b0b4"
    ]
  },
  {
    "id": "retro-70s",
    "name": "Retro 70s",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "Seventies film: warm browns, mustard yellows, faded blacks and grain.",
    "skinProtection": 40.0,
    "swatch": [
      "#8a5a28",
      "#d8b050"
    ]
  },
  {
    "id": "neon-rain",
    "name": "Neon Rain",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "Rain-soaked city nights: magenta and cyan neon, deep blacks, glowing lights.",
    "skinProtection": 45.0,
    "swatch": [
      "#d02a8a",
      "#18b8d8"
    ]
  },
  {
    "id": "war-epic",
    "name": "War Epic",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "Drained colour, gritty contrast and cold highlights, like a war film.",
    "skinProtection": 35.0,
    "swatch": [
      "#4a4c42",
      "#b0b0a4"
    ]
  },
  {
    "id": "romance-glow",
    "name": "Romance Glow",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "Warm, soft and dreamy: diffused highlights and gentle rose-gold tones.",
    "skinProtection": 50.0,
    "swatch": [
      "#e0a090",
      "#f4e0c8"
    ]
  },
  {
    "id": "arthouse-muted",
    "name": "Arthouse Muted",
    "world": "Cinematic & Film Emulation",
    "category": "Cinematic",
    "listed": true,
    "description": "Quiet, painterly independent-film palette: soft contrast, olive and dusty tones.",
    "skinProtection": 45.0,
    "swatch": [
      "#6a6a4a",
      "#c4b89a"
    ]
  },
  {
    "id": "silver-charcoal",
    "name": "Silver & Charcoal",
    "world": "Character & High-Contrast Portraiture",
    "category": "Legacy",
    "listed": false,
    "description": "Black and white with deep blacks and crisp midtone texture. Skin keeps its tonal separation instead of sinking into mud.",
    "skinProtection": 60.0,
    "swatch": [
      "#111214",
      "#d9dadc"
    ]
  },
  {
    "id": "volumetric-golden-hour",
    "name": "Volumetric Golden Hour",
    "world": "Atmospheric & Environmental Landscapes",
    "category": "Legacy",
    "listed": false,
    "description": "Warm low-sun light with soft bloom around the brights and a hazy, lifted atmosphere. Skin stays natural rather than orange.",
    "skinProtection": 75.0,
    "swatch": [
      "#5a3a1c",
      "#f2b45a"
    ]
  },
  {
    "id": "moody-earthy",
    "name": "Moody & Earthy",
    "world": "Atmospheric & Environmental Landscapes",
    "category": "Legacy",
    "listed": false,
    "description": "Muted foliage, deep slate blues and rich earth-tone shadows, with warm accents on skin.",
    "skinProtection": 70.0,
    "swatch": [
      "#2d2a22",
      "#6b7a5e"
    ]
  },
  {
    "id": "high-key-editorial",
    "name": "High-Key Editorial",
    "world": "High-Fashion & Melanin Precision",
    "category": "Legacy",
    "listed": false,
    "description": "Bright and clean: luminous skin, soft pastel shadows, gentle contrast.",
    "skinProtection": 60.0,
    "swatch": [
      "#d9d2e6",
      "#f7efe9"
    ]
  }
];

export function installMockBackend() {
  (globalThis as { isTauri?: boolean }).isTauri = true;
  mockWindows("main");
  mockIPC(async (cmd, args) => {
    const a = args as Record<string, unknown>;
    await new Promise((r) => setTimeout(r, 15)); // pretend IPC latency
    switch (cmd) {
      case "plugin:dialog|open": {
        // Open Photo asks for a file, Open Folder for a directory, Import LUT for a .cube.
        const o = a.options as { directory?: boolean; filters?: { extensions: string[] }[] } | undefined;
        if (o?.filters?.some((f) => f.extensions.includes("cube"))) return "/mock/luts/Kodak 2383.cube";
        return o?.directory === false ? `${FOLDER}/L1000583.DNG` : FOLDER;
      }
      case "list_luts":
        return luts;
      case "has_ai_key":
        return localStorage.getItem(`epikos.mock.aikey.${a.provider}`) === "1";
      case "save_ai_key":
        if (String(a.key ?? "").length < 20) throw new Error("That doesn't look like an API key");
        localStorage.setItem(`epikos.mock.aikey.${a.provider}`, "1"); // never the key itself
        return null;
      case "delete_ai_key":
        localStorage.removeItem(`epikos.mock.aikey.${a.provider}`);
        return null;
      case "interpret_look_ai":
        throw new Error("the browser mock has no AI service");
      case "learned_styles":
        return load(LEARNED_KEY);
      case "learn_style": {
        await new Promise((r) => setTimeout(r, 700));
        const src = String(a.path).split("/").pop()!;
        const name = String(a.name || "").trim() || src.replace(/\.[^.]+$/, "");
        const created = Math.floor(Date.now() / 1000);
        const style = {
          id: `learned-${name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}-${created}`,
          name,
          source: src,
          createdAt: created,
          signature: {
            tone: { black: 0.12, white: 0.96, median: 0.46, contrast: 0.29 },
            skin: { l: 0.42, hue: 56, chroma: 0.07, richness: 0.7, relativeL: -0.02, specular: 0.05 },
            bands: [],
            foliage: { hue: 118, chroma: 0.06, share: 0.1 },
            backgroundChroma: 0.03,
          },
          palette: [
            { hex: "#2b2622", weight: 0.3 },
            { hex: "#6b4a36", weight: 0.22 },
            { hex: "#a07b5c", weight: 0.2 },
            { hex: "#556048", weight: 0.16 },
            { hex: "#e6ddd2", weight: 0.12 },
          ],
        };
        store(LEARNED_KEY, [...load(LEARNED_KEY), style]);
        return style;
      }
      case "delete_learned_style":
        store(LEARNED_KEY, load<{ id: string }>(LEARNED_KEY).filter((s) => s.id !== a.id));
        return null;
      case "list_presets":
        return load(PRESETS_KEY);
      case "save_preset": {
        const created = Date.now();
        const preset = { id: `preset-${created}`, name: String(a.name || "").trim() || "My preset", createdAt: Math.floor(created / 1000), adjustments: a.adjustments };
        store(PRESETS_KEY, [...load(PRESETS_KEY), preset]);
        return preset;
      }
      case "delete_preset":
        store(PRESETS_KEY, load<{ id: string }>(PRESETS_KEY).filter((p) => p.id !== a.id));
        return null;
      case "import_lut": {
        const name = String(a.path).split("/").pop()!.replace(/\.cube$/i, "");
        if (!luts.some((l) => l.name === name)) luts.push({ name, title: name, size: 33 });
        return luts.find((l) => l.name === name);
      }
      case "remove_lut":
        luts.splice(luts.findIndex((l) => l.name === a.name), 1);
        return null;
      case "mentor": {
        await new Promise((r) => setTimeout(r, 400));
        const adj = structuredClone(a.adjustments as Adjustments);
        // Like the engine: only what the current edit still needs.
        const insights: { topic: string; observation: string; why: string; how: string }[] = [];
        const changes: string[] = [];
        const recommended: Adjustments = structuredClone(adj);
        const upsert = (mask: LocalAdjustment["mask"], patch: Partial<LocalAdjustment>) => {
          const l = recommended.local.find((x) => x.mask === mask);
          if (l) Object.assign(l, patch);
          else recommended.local.push({ ...defaultLocal(mask), ...patch });
        };
        if (Math.abs(adj.lens.rotation - 0.69) > 0.3) {
          insights.push({
            topic: "Framing",
            observation: "The lines in the frame lean and converge (+0.7° tilt, +19 vertical perspective).",
            why: "A tilted horizon or falling buildings read as a mistake and pull the eye to the edges.",
            how: "Step 1: Auto upright, then fine-tune Straighten if the subject itself leans on purpose.",
          });
          recommended.lens = { ...adj.lens, rotation: 0.69, vertical: 19 };
          changes.push("Straighten +0.69°, vertical +19");
        }
        if (Math.abs(adj.exposure - 1.24) >= 0.3) {
          insights.push({
            topic: "Exposure",
            observation: `The mid-tones sit about ${Math.abs(1.24 - adj.exposure).toFixed(1)} stops ${adj.exposure < 1.24 ? "below" : "above"} mid-grey.`,
            why: "Exposure sets where every other decision starts. This reading uses the whole frame, not the skin, so deep skin keeps its natural depth.",
            how: "Step 2: Exposure to +1.24 EV (or Auto exposure & tone).",
          });
          recommended.exposure = 1.24;
          recommended.tone = { ...adj.tone, highlights: -42, blacks: -8 };
          changes.push("Exposure +1.24 EV", "Highlights -42", "Blacks -8");
        }
        const skin = adj.local.find((l) => l.mask === "skin");
        if (!skin || skin.warmth < 10) {
          insights.push({
            topic: "Skin",
            observation: "Skin in the current edit reads grey / ashy (too little colour).",
            why: "Deep skin turns grey under cool light or a cool grade; its richness is warmth and colour, not brightness, so the fix is colour on the skin alone.",
            how: "Step 3: local adjustment on Skin, warmth +15, tint +5, saturation +10 (measured on this photo's skin).",
          });
          upsert("skin", { warmth: 15, tint: 5, saturation: 10 });
          changes.push("Skin warmth +15, tint +5, saturation +10");
        }
        if (!adj.local.some((l) => l.mask === "subject")) {
          insights.push({
            topic: "Subject",
            observation: "The subject is 1.1 stops darker than its surroundings.",
            why: "The eye goes to the brightest part of a picture first; a subject darker than the background competes with it.",
            how: "Step 3: local adjustment on Subject, about +0.45 EV (rather than brightening everything).",
          });
          upsert("subject", { exposure: 0.45 });
          changes.push("Subject +0.45 EV");
        }
        if (!adj.local.some((l) => l.mask === "background")) {
          insights.push({
            topic: "Subject",
            observation: "The background is warm, close to the skin's own colour.",
            why: "Skin separates best against cooler, quieter colour; a warm background blends the person into it.",
            how: "Step 3: local adjustment on Background, warmth about −20 and saturation −10.",
          });
          upsert("background", { warmth: -20, saturation: -10 });
          changes.push("Background cooler (warmth −20, saturation −10)");
        }
        // The starting target (Editorial or a learned look), as the engine fits it.
        const learnedTarget = a.target ? load<{ id: string; name: string }>(LEARNED_KEY).find((s) => s.id === a.target) : undefined;
        const targetName = learnedTarget?.name ?? "Editorial";
        const fits: string[] = [];
        if (recommended.tone.blacks > -30) {
          recommended.tone.blacks = -30;
          fits.push("Blacks -30 (black point)");
        }
        if (!recommended.curves.sCurve.enabled) {
          recommended.curves = { ...recommended.curves, sCurve: { ...recommended.curves.sCurve, enabled: true, amount: 28 } };
          fits.push("Midtone S-curve 28");
        }
        if (recommended.color.foliage.hue < 12) {
          recommended.color = { ...recommended.color, foliage: { hue: 12, saturation: -18, luminance: 0 } };
          fits.push("Foliage hue +12 (towards olive), saturation -18");
        }
        if (fits.length) {
          insights.push({
            topic: "Style",
            observation: learnedTarget
              ? `Starting point aimed at your learned style "${targetName}": its black and white points, midtone curve, skin richness and colour.`
              : "Starting point aimed at the Editorial profile: rich, anchored blacks, a clean white point, a midtone S-curve, skin rich and warm with its highlights kept, and foliage calmed towards olive.",
            why: "Contrast and depth read as professional; lifting every shadow reads as flat HDR.",
            how: `Apply Recommended Starting Point: ${fits.join(", ")}.`,
          });
          changes.push(...fits);
        }
        const cropped = adj.crop && (adj.crop.width < 0.999 || adj.crop.height < 0.999);
        const crop = cropped
          ? null
          : {
              crop: { x: 0.12, y: 0.08, width: 0.8, height: 0.8, aspect: "original" as const },
              rotation: 0.69,
              reason: "puts the subject on the upper-left third, sets the horizon on the lower third line, keeps 80% of the frame",
            };
        if (crop) {
          insights.push({
            topic: "Framing",
            observation: `A tighter crop ${crop.reason}.`,
            why: "Placing the subject and the horizon on the thirds gives the picture direction and room.",
            how: "Apply suggested crop below, or Step 1: Crop & straighten at +0.7°.",
          });
        }
        return {
          summary: insights.length ? `Close-up Portrait · first priority: ${insights[0].topic.toLowerCase()}` : "Close-up Portrait: well exposed and level, a good base to grade from",
          insights,
          recommended,
          changes,
          crop,
          target: targetName,
          analysisMs: 1400,
        };
      }
      case "critique": {
        const adj = a.adjustments as Adjustments;
        const out: { level: string; text: string; fix: { label: string; adjustments: Adjustments } | null }[] = [];
        if (adj.exposure > 2) {
          const fixed = { ...adj, exposure: Math.round((adj.exposure - 0.25) * 100) / 100, tone: { ...adj.tone, highlights: Math.max(-100, adj.tone.highlights - 25) } };
          const label = `Highlights ${fixed.tone.highlights}, Exposure ${fixed.exposure >= 0 ? "+" : ""}${fixed.exposure.toFixed(2)} EV`;
          out.push({ level: "warning", text: `2.4% of the image is clipping to white. Fix: ${label}.`, fix: { label, adjustments: fixed } });
        }
        if (adj.tone.saturation > 30) {
          const fixed = { ...adj, tone: { ...adj.tone, saturation: adj.tone.saturation - 15, vibrance: adj.tone.vibrance - 10 } };
          const label = `Saturation +${fixed.tone.saturation}, Vibrance ${fixed.tone.vibrance}`;
          out.push({ level: "warning", text: `11% of the image is very saturated. Fix: ${label}.`, fix: { label, adjustments: fixed } });
        }
        if (adj.style.id) out.push({ level: "praise", text: "Skin tones are balanced: natural hue and depth, kept through the grade.", fix: null });
        if (adj.lens.rotation) out.push({ level: "praise", text: "Framing straightened: lines now read as intentional.", fix: null });
        if (out.length === 0) out.push({ level: "praise", text: "Nothing to flag: highlights, shadows and colour are all in a healthy range.", fix: null });
        return out;
      }
      case "photo_extensions":
        return ["arw", "srf", "sr2", "cr3", "cr2", "crw", "nef", "nrw", "raf", "dng", "jpg", "jpeg", "png", "tif", "tiff"];
      case "list_files":
        return filesFor(a.paths as string[]);
      case "story_arc_files":
        return storyFor(filesFor(a.paths as string[]).map((f) => f.path));
      case "plugin:dialog|save":
        return (a.options as { defaultPath?: string } | undefined)?.defaultPath ?? `${FOLDER}/export.tif`;
      case "airdrop_export":
        await new Promise((r) => setTimeout(r, 700));
        return {
          path: `/tmp/EPIKOS RAW AirDrop/${String(a.path).split("/").pop()!.replace(/\.[^.]+$/, "")}.jpg`,
          format: (a.options as { format: string }).format,
          width: 7728,
          height: 5152,
          colorSpace: "sRGB IEC61966-2.1",
          bytes: 9_800_000,
          wroteExif: true,
          wroteLocation: false,
          alphaChannels: [],
          developMs: 1100,
          writeMs: 60,
        };
      case "export_image":
        await new Promise((r) => setTimeout(r, 900));
        console.info("[mock] export", a);
        return {
          path: a.dest,
          format: (a.options as { format: string }).format,
          width: 7728,
          height: 5152,
          colorSpace: (a.options as { colorSpace: string }).colorSpace,
          bytes: 231_900_000,
          wroteExif: true,
          wroteLocation: (a.options as { includeLocation: boolean }).includeLocation,
          alphaChannels: (a.options as { aiMasks: boolean }).aiMasks ? ["Subject", "Sky", "Skin"] : [],
          // (the dialog never asks a JPEG or PNG for masks)
          developMs: 1180,
          writeMs: 850,
        };
      case "list_folder":
        return FILES.map((f) => ({ ...f, hasEdits: f.hasEdits || saved.has(f.path) }));
      case "open_image":
        return info(a.path as string);
      case "render_preview":
        return render(a.adjustments as Adjustments, a.maxWidth as number, a.maxHeight as number, a.path as string);
      case "thumbnail":
        return jpegThumb(a.path as string);
      case "handoff_apps":
        return [
          { name: "Adobe Photoshop 2026", path: "/Applications/Adobe Photoshop 2026/Adobe Photoshop 2026.app" },
          { name: "Adobe Lightroom Classic", path: "/Applications/Adobe Lightroom Classic/Adobe Lightroom Classic.app" },
        ];
      case "open_in_app":
        console.info("[mock] open", a.file, "in", a.app);
        return null;
      case "interpret_look": {
        // Stand-in for the engine's interpreter: recognises a few words only.
        const adj = structuredClone(a.adjustments as Adjustments);
        const words = String(a.prompt).toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
        const matched: { phrase: string; effect: string; strength: number }[] = [];
        if (words.includes("foggy")) {
          adj.atmosphere.fog += 45;
          matched.push({ phrase: "foggy", effect: "distance fog", strength: 1 });
        }
        if (words.includes("film")) {
          adj.finishing.grain += 30;
          matched.push({ phrase: "film", effect: "film: grain, soft blacks", strength: 1 });
        }
        const known = new Set(["foggy", "film", "a", "an", "the", "with", "make", "this", "look", "like"]);
        return { adjustments: adj, matched, unknown: words.filter((w) => !known.has(w)) };
      }
      case "analyze_image":
        await new Promise((r) => setTimeout(r, 400));
        return analysis(a.path as string);
      case "story_arc":
        return story();
      case "sync_look": {
        const targets = a.targets as string[];
        const report: SyncReport = { frames: [], skipped: [] };
        targets.forEach((path, i) => {
          if (saved.has(path)) presync.set(path, saved.get(path)!);
          else presync.set(path, null);
          saved.set(path, { version: 1, adjustments: structuredClone(a.adjustments as Adjustments) } as DevelopDocument);
          report.frames.push({
            path,
            exposureDelta: [0.35, -0.6, 0.1][i % 3],
            whiteBalanceShifted: i % 2 === 0,
            textureScale: [1, 1.22, 0.84][i % 3],
            skinProtection: 70,
          });
        });
        return report;
      }
      case "undo_sync": {
        const restored: string[] = [];
        for (const path of a.targets as string[]) {
          if (!presync.has(path)) continue;
          const before = presync.get(path);
          if (before) saved.set(path, before);
          else saved.delete(path);
          presync.delete(path);
          restored.push(path);
        }
        return restored;
      }
      case "list_styles":
        return STYLES;
      case "mask_models":
        return {
          dir: "/mock/models",
          models: [
            { kind: "subject", file: "/mock/models/isnet-general-use.onnx", available: true },
            { kind: "sky", file: "/mock/models/skyseg.onnx", available: true },
          ],
          depth: { file: "/mock/models/depth-anything-v2-small.onnx", available: true },
          face: { file: "/mock/models/face-parsing-resnet18.onnx", available: true },
          inpaint: { file: "/mock/models/lama-fp32.onnx", available: true },
          targets: (
            ["subject", "background", "sky", "skin", "eyes", "hair", "foreground", "people", "vegetation", "clothing", "lips", "glasses"] as MaskTarget[]
          ).map(
            (target) => ({ target, available: true, source: "mock" }),
          ),
          lensDatabase: 1569,
        };
      case "detect_mask":
        await new Promise((r) => setTimeout(r, 300));
        return mask(a.kind as MaskTarget);
      case "local_mask":
        await new Promise((r) => setTimeout(r, 200));
        return mask((a.adjustments as Adjustments).local[a.index as number].mask);
      case "find_dust_spots":
        await new Promise((r) => setTimeout(r, 300));
        return [
          { x: 0.83, y: 0.12, radius: 0.006 },
          { x: 0.91, y: 0.3, radius: 0.004 },
          { x: 0.12, y: 0.07, radius: 0.005 },
        ];
      case "auto_tone":
        await new Promise((r) => setTimeout(r, 200));
        return {
          exposure: 1.24,
          tone: { contrast: 0, highlights: -66, shadows: 0, whites: 0, blacks: -13, vibrance: 10, saturation: 0 },
        };
      case "auto_upright":
        await new Promise((r) => setTimeout(r, 200));
        return { rotation: 0.69, vertical: 19 };
      case "detect_depth":
        await new Promise((r) => setTimeout(r, 300));
        return depthMap();
      case "save_document":
        saved.set(a.path as string, a.document as DevelopDocument);
        console.info("[mock] saved", a.path, a.document);
        return { jsonPath: `${a.path}.epikos.json`, xmpPath: null, warning: null };
      default:
        throw new Error(`mock backend: unhandled command ${cmd}`);
    }
  }, { shouldMockEvents: true });
  // Dev only: simulate files dropped on the window, e.g.
  // __epikosDrop(["/mock/Sydney shoot/L1000583.DNG", "/tmp/notes.txt"]).
  (globalThis as { __epikosDrop?: (paths: string[]) => Promise<void> }).__epikosDrop = async (paths) => {
    await emit("tauri://drag-enter", { paths, position: { x: 400, y: 300 } });
    await new Promise((r) => setTimeout(r, 400));
    await emit("tauri://drag-drop", { paths, position: { x: 400, y: 300 } });
  };
}

function info(path: string): ImageInfo {
  const f = FILES.find((x) => x.path === path)!;
  return {
    path,
    name: f.name,
    format: f.format,
    make: f.format.split(" ")[0],
    model: f.format.startsWith("Fuji") ? "X-T5" : f.format.startsWith("Leica") ? "SL2" : "EOS R5",
    width: 7728,
    height: 5152,
    monochrome: false,
    asShot: AS_SHOT,
    capture: {
      make: f.format.split(" ")[0].toUpperCase(),
      model: "X-T5",
      dateTimeOriginal: "2026:06:21 04:13:51",
      exposureTime: [1, 45],
      fNumber: [4, 1],
      focalLength: [302, 10],
      exposureBias: [0, 1],
      iso: 8000,
      lensMake: "FUJIFILM",
      lensModel: "XF16-55mmF2.8 R LM WR",
      gps: f.name.startsWith("L") ? { latitude: [[1, 1], [2, 1], [3, 1]], longitude: [[4, 1], [5, 1], [6, 1]] } : null,
    },
    document: saved.get(path) ?? {
      version: 1,
      source: { path, sha256: "", format: f.format, make: "", model: "" },
      adjustments: defaultAdjustments(),
    },
    loadedFrom: saved.has(path) ? `${path}.epikos.json` : null,
    lensProfile: f.name.endsWith("JPG")
      ? null
      : f.format.startsWith("Leica")
        ? "Camera (DNG)"
        : f.format.startsWith("Fuji")
          ? "Lensfun: Fujifilm XF16-55mmF2.8 R LM WR"
          : null,
    bitmap: f.name.endsWith("JPG"),
  };
}

function seed(path: string) {
  let h = 0;
  for (const c of path) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return (h % 360) / 360;
}

/** Linear-light synthetic scene: sky gradient, horizon, colour patches, grey ramp. */
function scene(u: number, v: number, hue: number): [number, number, number] {
  if (v > 0.82) {
    const g = 0.02 + 0.2 * u; // grey ramp
    return [g, g, g];
  }
  if (v > 0.55 && v < 0.75 && u > 0.08 && u < 0.92) {
    const i = Math.floor(((u - 0.08) / 0.84) * 8);
    const h = (hue + i / 8) % 1;
    const [r, g, b] = hsv(h, 0.65, 0.45);
    return [r, g, b];
  }
  if (v < 0.55) {
    const t = v / 0.55;
    return [0.12 + 0.35 * t, 0.2 + 0.3 * t, 0.45 + 0.1 * t]; // sky
  }
  return [0.05, 0.06, 0.04];
}

function hsv(h: number, s: number, v: number): [number, number, number] {
  const f = (n: number) => {
    const k = (n + h * 6) % 6;
    return v - v * s * Math.max(0, Math.min(k, 4 - k, 1));
  };
  return [f(5), f(3), f(1)];
}

function render(adj: Adjustments, maxW: number, maxH: number, path: string): ArrayBuffer {
  const crop = adj.crop ?? { x: 0, y: 0, width: 1, height: 1 };
  const aspect = (3 / 2) * (crop.width / crop.height);
  let w = Math.min(maxW, 1600);
  let h = Math.round(w / aspect);
  if (h > maxH) {
    h = Math.min(maxH, 1067);
    w = Math.round(h * aspect);
  }
  const gain = Math.pow(2, adj.exposure);
  const wb = adj.whiteBalance.mode === "custom" ? adj.whiteBalance : { ...AS_SHOT };
  const t = (wb.temperature - AS_SHOT.temperature) / 4000;
  const tint = (wb.tint - AS_SHOT.tint) / 300;
  const mul = [1 + 0.35 * t + tint, 1 - tint, 1 - 0.35 * t + tint];
  const hue = seed(path);
  // Rough stand-ins for the styles: enough to see the UI react, not the real looks.
  const k = adj.style?.id ? (adj.style.amount ?? 100) / 100 : 0;
  const mono = adj.style?.id === "silver-charcoal" ? k : 0;
  const warm =
    adj.style?.id === "volumetric-golden-hour" ? [1 + 0.25 * k, 1 + 0.05 * k, 1 - 0.3 * k]
    : adj.style?.id === "dark-melanin-glow" ? [1 + 0.08 * k, 1, 1 - 0.1 * k]
    : [1, 1, 1];

  const hist = new Uint32Array(3 * 256);
  const out = new ArrayBuffer(8 + hist.byteLength + w * h * 4);
  const view = new DataView(out);
  view.setUint32(0, w, true);
  view.setUint32(4, h, true);
  const px = new Uint8ClampedArray(out, 8 + hist.byteLength);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const raw = scene(crop.x + (x / w) * crop.width, crop.y + (y / h) * crop.height, hue);
      const grey = 0.2627 * raw[0] + 0.678 * raw[1] + 0.0593 * raw[2];
      const lin = raw.map((v, c) => (v + (grey - v) * mono) * warm[c]);
      const i = (y * w + x) * 4;
      for (let c = 0; c < 3; c++) {
        const v = Math.max(0, lin[c] * gain * mul[c]);
        const shoulder = v <= 0.8 ? v : 0.8 + 0.2 * (1 - Math.exp(-(v - 0.8) / 0.2));
        const e = shoulder <= 0.0031308 ? 12.92 * shoulder : 1.055 * Math.pow(shoulder, 1 / 2.4) - 0.055;
        const b = Math.round(Math.min(1, e) * 255);
        px[i + c] = b;
        hist[c * 256 + b]++;
      }
      px[i + 3] = 255;
    }
  }
  new Uint32Array(out, 8, 3 * 256).set(hist);
  return out;
}

/** Sky = the synthetic sky gradient; subject = the colour-patch row. */
function mask(kind: MaskTarget): ArrayBuffer {
  const w = 1024;
  const h = Math.round(w / 1.5);
  const out = new ArrayBuffer(12 + w * h);
  const view = new DataView(out);
  view.setUint32(0, w, true);
  view.setUint32(4, h, true);
  view.setUint32(8, kind === "sky" ? 380 : 900, true);
  const alpha = new Uint8Array(out, 12);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const [u, v] = [x / w, y / h];
      const subject = v > 0.55 && v < 0.75 && u > 0.08 && u < 0.92;
      const inside =
        kind === "sky"
          ? v < 0.55
          : kind === "background"
            ? !subject
            : kind === "foreground"
              ? v > 0.7
              : kind === "eyes"
                ? v > 0.6 && v < 0.62 && ((u > 0.45 && u < 0.47) || (u > 0.52 && u < 0.54))
                : kind === "eyebrows"
                  ? v > 0.585 && v < 0.595 && ((u > 0.445 && u < 0.475) || (u > 0.515 && u < 0.545))
                  : kind === "eyelashes"
                    ? v > 0.598 && v < 0.602 && ((u > 0.45 && u < 0.47) || (u > 0.52 && u < 0.54))
                    : kind === "teeth"
                      ? v > 0.655 && v < 0.665 && u > 0.48 && u < 0.52
                      : kind === "faceSkin"
                        ? v > 0.58 && v < 0.68 && u > 0.44 && u < 0.56
                        : kind === "bodySkin"
                          ? v > 0.68 && v < 0.75 && u > 0.46 && u < 0.54
                : kind === "hair"
                  ? v > 0.55 && v < 0.58 && u > 0.44 && u < 0.56
                  : kind === "skin"
                    ? v > 0.58 && v < 0.68 && u > 0.43 && u < 0.57
                    : subject;
      alpha[y * w + x] = inside ? 255 : 0;
    }
  }
  return out;
}

async function jpegThumb(path: string): Promise<ArrayBuffer> {
  const buf = render(defaultAdjustments(), 240, 160, path);
  const w = new DataView(buf).getUint32(0, true);
  const h = new DataView(buf).getUint32(4, true);
  const canvas = new OffscreenCanvas(w, h);
  canvas.getContext("2d")!.putImageData(new ImageData(new Uint8ClampedArray(buf, 8 + 3 * 256 * 4), w, h), 0, 0);
  const blob = await canvas.convertToBlob({ type: "image/jpeg", quality: 0.8 });
  return blob.arrayBuffer();
}

/** Sky far (0), ground getting nearer towards the bottom, patches in between. */
function depthMap(): ArrayBuffer {
  const w = 1024;
  const h = Math.round(w / 1.5);
  const out = new ArrayBuffer(12 + w * h);
  const view = new DataView(out);
  view.setUint32(0, w, true);
  view.setUint32(4, h, true);
  view.setUint32(8, 150, true);
  const d = new Uint8Array(out, 12);
  for (let y = 0; y < h; y++) {
    const v = y / h;
    const near = v < 0.55 ? 0 : v > 0.55 && v < 0.75 ? 0.6 : Math.min(1, (v - 0.55) / 0.45);
    d.fill(Math.round(near * 255), y * w, (y + 1) * w);
  }
  return out;
}

// Shapes and values from `epikos analyze` / `epikos story` on the real test photos.
function analysis(path: string): SceneAnalysis {
  const portrait = /L1000583|IMG_0421/.test(path);
  const bird = /DSCF/.test(path);
  return {
    genres: portrait
      ? [
          { id: "environmental-portrait", label: "Environmental Portrait", score: 0.95, evidence: "skin 7.2% in a wider scene, depth range 0.94" },
          { id: "dark-melanin-fashion", label: "Dark Melanin Fashion", score: 0.95, evidence: "portrait with skin tone depth ~8/10" },
          { id: "close-up-portrait", label: "Close-up Portrait", score: 0.93, evidence: "skin 7.2%, subject 29.3%" },
        ]
      : bird
        ? [{ id: "wildlife", label: "Wildlife", score: 0.46, evidence: "no people, 140 mm, isolated subject 1.1%, 15.4% foliage / earth colours" }]
        : [{ id: "environmental-portrait", label: "Environmental Portrait", score: 0.72, evidence: "skin 0.7% in a wider scene, depth range 0.95" }],
    lighting: {
      colorTemperature: bird ? 4239 : 6156,
      ambientTemperature: bird ? 4063 : 6025,
      ambientLabel: bird ? "golden" : "daylight",
      dynamicRangeEv: 7.5,
      highlightsClipped: 0.4,
      shadowsCrushed: 0.1,
      key: "mid-key",
      hardness: portrait ? 0.89 : 0.58,
      hardnessLabel: portrait ? "hard, direct" : "medium",
      direction: "from the upper right",
      backlit: portrait,
      haze: 0.08,
      hazeLabel: "clear",
      snow: 0,
      timeOfDay: "daytime",
    },
    skin: bird
      ? null
      : {
          coverage: portrait ? 7.2 : 0.7,
          toneDepth: portrait ? 8 : 8.4,
          toneLabel: "deep",
          undertone: "neutral",
          shine: 0.68,
          shineLabel: portrait ? "glossy" : "too small to judge",
          texture: 0.2,
          textureLabel: portrait ? "smooth" : "too small to judge",
        },
    composition: { subjectCoverage: portrait ? 29.3 : bird ? 1.1 : 4.7, skyCoverage: bird ? 1.6 : 12, depthRange: 0.9, lineStrength: 0.1 },
    palette: [
      { hex: "#67675d", weight: 0.24 },
      { hex: "#373b39", weight: 0.21 },
      { hex: "#8e7a6a", weight: 0.2 },
      { hex: "#1f1a18", weight: 0.19 },
      { hex: "#c9c3b8", weight: 0.16 },
    ],
    luminance: {
      // A mid-key frame: a broad hump around 35%, a little sky near white.
      histogram: Array.from({ length: 64 }, (_, i) => Math.exp(-(((i - 22) / 10) ** 2)) / 17.7 + (i > 56 ? 0.004 : 0)),
      mean: 0.36,
      median: 0.34,
    },
    limits: [],
    analysisMs: 3100,
  };
}

/** The mock's photos among `paths` (a folder path selects all of them). */
function filesFor(paths: string[]): FileEntry[] {
  if (paths.includes(FOLDER)) return FILES.map((f) => ({ ...f, hasEdits: f.hasEdits || saved.has(f.path) }));
  return FILES.filter((f) => paths.includes(f.path)).map((f) => ({ ...f, hasEdits: f.hasEdits || saved.has(f.path) }));
}

/** The folder's story arc, restricted to `paths`. */
function storyFor(paths: string[]): StoryArc {
  const all = story();
  const groups = all.groups
    .map((g) => ({ ...g, frames: g.frames.filter((p) => paths.includes(p)) }))
    .filter((g) => g.frames.length > 0)
    .map((g, id) => ({
      ...g,
      id,
      label: g.label.replace(/\d+ photos?$/, `${g.frames.length} photo${g.frames.length === 1 ? "" : "s"}`),
      hero: g.frames.includes(g.hero) ? g.hero : g.frames[0],
    }));
  return { ...all, groups, frames: all.frames.filter((f) => paths.includes(f.path)) };
}

function story(): StoryArc {
  const groups = [
    { names: ["DSCF0346.RAF", "DSCF0352.RAF"], label: "2025-12-14 15:41–15:52 · 2 photos", reason: "", palette: ["#5a5555", "#97818c", "#2e2f30", "#5a874d", "#beceb0"] },
    { names: ["L1000530.DNG"], label: "2026-08-17 08:18 · 1 photo", reason: "245 days without shooting", palette: ["#4f5157", "#95a6b6", "#648db0", "#2e3032", "#cac9c6"] },
    { names: ["L1000583.DNG", "IMG_0421.CR3"], label: "2026-09-20 12:04–12:09 · 2 photos", reason: "34 days without shooting", palette: ["#67675d", "#373b39", "#8e7a6a", "#1f1a18", "#c9c3b8"] },
  ];
  return {
    groups: groups.map((g, id) => ({
      id,
      label: g.label,
      frames: g.names.map((n) => `${FOLDER}/${n}`),
      hero: `${FOLDER}/${g.names[0]}`,
      palette: g.palette.map((hex, i) => ({ hex, weight: [0.29, 0.2, 0.19, 0.17, 0.15][i] })),
      splitReason: g.reason,
    })),
    frames: FILES.map((f) => ({
      path: f.path,
      name: f.name,
      captured: null,
      sceneEv: 10,
      lightness: 0.5,
      cast: [0, 0],
      gps: null,
      skin: 0,
      group: groups.findIndex((g) => g.names.includes(f.name)),
      error: null,
    })),
    analysisMs: 113,
  };
}
