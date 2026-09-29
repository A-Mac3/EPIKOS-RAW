# EPIKOS RAW

Cross-platform RAW/DNG editor (JPEG, PNG and TIFF too). Product spec: [PRD.md](PRD.md).

Open a folder (⌘O), a single photo (⇧⌘O), or drop photos or folders anywhere on the
window; photos opened one by one get the same filmstrip, masks and exports as a folder.

The left panel (◧) holds **Presets & Styles** (over 120 styles in eight tabs: Film
Simulations, with looks inspired by every Fujifilm film simulation and the Leica looks,
Portraits & Skin, Landscape & Nature, Aerial, Lifestyle, Essentials, Macro and
Cinematic; the Style Fusion Matrix with any four styles in its corners, and imported `.cube` 3D LUTs; rest the
pointer on a card to preview it), the **AI Mentor** (a rule-based reading of the photo
that re-reads the edit whenever it pauses: a recommended starting point with global and
local moves, skin balance judged for each skin's own depth, subject lift and background
cooling, and a suggested crop and straighten) and **History** (every step, clickable).
**Split** (Y) wipes between before and after. **Crop & straighten** (C) has aspect presets
(Free, Original, 1:1, 4:5, 16:9, 9:16), a rule-of-thirds grid and a straighten knob. Scroll
or pinch over the image to zoom (10–500 % of the actual pixels); drag, or hold Space and
drag, to pan. The Scene panel shows the photo's as-shot palette next to a live palette of
the current edit, its luminance histogram and the light's measured colour temperature.
**Learn Style from Photo** (Presets & Styles) measures a reference photo's look: black
and white points and midtone curve, skin richness within the skin's own depth and its
specular highlights, per-band colour, foliage and background saturation. The AI Mentor's
starting point aims at a pro profile for the photo's genre (portrait, deep-skin portrait,
light & airy wedding, landscape, street, architecture, wildlife, underwater, night &
astro, or by choice aerial, macro and product & food: where photographers in that genre
typically set the black and white points, midtone contrast, skin and greenery), the
built-in Editorial profile, or any learned style (your own references), fitted on the
photo's own render. Learned styles and custom presets are kept in
`~/.epikos/learned_styles.json` and `~/.epikos/presets.json` (or `$EPIKOS_DATA_DIR`), so
they persist across sessions and updates.
Step 3's AI masks include people, vehicles and animals (FCN-ResNet50 scene
segmentation), vegetation and foliage, clothing, facial skin, body skin, eyes, eyebrows,
eyelashes (the lash line at the eyes' edge), teeth, lips, facial hair, glasses and hair.
Selecting a local adjustment highlights its mask for review; the highlight clears as
soon as an edit starts, and Show mask brings it back. Each has exposure, contrast,
highlights, shadows, whites, blacks, saturation, warmth, tint and clarity. Each mask can
be extended or reduced, feathered (25 by default, so edits fade in without a visible edge) and brushed
where it should (Add) or shouldn't (Remove) reach. Face features are parsed at twice the mask
resolution; skin is kept to the person (on the subject and matching their own skin, so
walls and clothing never count); where the hair model finds almost none (very dark or
close-cropped hair, a beard) hair comes from the head's darker, textured areas. Step 4
has Teeth whitening and **Retouch**: Generative erase (paint over something, then
Erase; the LaMa inpainting model fills it from its surroundings, once, and exposure,
tone and the look apply to the fill like the rest of the photo), Heal spots (click a
spot, or Find dust spots to have faint, round, isolated specks on smooth areas circled
for review) and Red-eye removal. Step 2 has a quick Vignette slider (the full vignette
controls are in Step 8). Mask overlays hide while any slider is dragged and return on
release.
Step 3 also has **Manual masking**: a brush (size, feather, flow, erase), linear
(graduated) and radial gradients drawn on the image, each with exposure, contrast,
highlights, shadows, whites, blacks, temperature, tint, dehaze, saturation and clarity. While a control is dragged the preview
renders a lighter proxy so feedback keeps up, then sharpens when the drag ends. The AI
Mentor ranks subjects (primary / secondary, by area and nearness), frames people by
portrait rules (never cutting the neck, waist, knees or ankles; full body with ground
below the feet or a clean three-quarter; headroom and lead room) and gives every warning
a one-click **Fix** with its slider targets. The Export dialog can **AirDrop** a JPEG,
PNG or TIFF (macOS). "Describe a look" understands tone-curve and HSL phrases ("deep
blacks, creamy highlights, teal shadows, desaturated greens"); in Settings (⚙) it can
optionally use your own Anthropic or OpenAI key, stored in the macOS Keychain and sent
only with the prompt text.
Edits save automatically
to `<photo>.epikos.json` (and an Adobe-compatible `.xmp`) and come back when the photo
is reopened.

## Layout

| Path | What |
|---|---|
| `crates/epikos-core` | 32-bit float image types, camera formats, sensor profiles, lens-profile models |
| `crates/epikos-decode` | RAW/DNG decode via `rawler` (plus the DNG's own lens corrections from `OpcodeList3`), JPEG/PNG/TIFF decode (TIFFs with extra mask channels included), crop, embedded thumbnails |
| `crates/epikos-pipeline` | Demosaic, highlights, white balance, noise reduction, lens profiles and optics, colour, exposure, Step 2 tone and auto-tone, straighten / vertical perspective and auto-upright, Step 3 local mask adjustments, Step 4 texture / retouching, Step 5 HSL, colour wheels, foliage shift and background re-colouration, Step 6 glow / depth fog / light shafts, Step 7 parametric and point curves, split toning, Step 8 grain and vignette, 3D virtual lights, parametric styles and style fusion (`src/look`), preview binning, display transform |
| `crates/epikos-sidecar` | Non-destructive `.epikos.json` (canonical) and Adobe-compatible `.xmp` |
| `crates/epikos-masks` | On-device models via ONNX Runtime (CPU): IS-Net subject and skyseg sky masks, BiSeNet face parsing for eyes and hair (Step 3), Depth Anything V2 Small depth (Steps 3 and 6) |
| `crates/epikos-engine` | Session layer for front-ends: image cache, previews, sidecar policy, Step 3 masks (subject, background, sky, skin, eyes, hair, foreground), the Lensfun lens database (`data/lensfun`, CC BY-SA 3.0), TIFF / layered PSD / enhanced DNG / JPEG / PNG export, natural-language look prompts, scene analysis and story-arc sync |
| `crates/epikos-cli` | `epikos inspect / sidecar / develop / export / analyze / story / mentor / styles` |
| `apps/desktop` | Tauri 2 + React/TypeScript desktop app (`src-tauri` = Rust commands) |

## Develop

Requires Rust (stable) and Node.js 20+.

```bash
cargo test --workspace                 # engine tests
scripts/fetch-models.sh                # once: ~455 MB of models into models/ (git-ignored)
cd apps/desktop && npm install         # once
npm run tauri dev                      # desktop app with hot reload
```

UI-only work without the engine: `npm run dev`, then open `http://localhost:5173/?mock`
(synthetic test chart, dev builds only).

Render a real file through the engine and time it:

```bash
cargo run --release -p epikos-engine --example render_preview -- <RAW> <OUT_DIR> [EV]
```

Render a file with each built-in style and a Step 6 sample (plus its skin and depth maps) and time full-resolution develops:

```bash
cargo run --release -p epikos-engine --example render_looks -- <RAW> <OUT_DIR> [--full]
```

Export (format from the extension). With `--masks` / `--depth` the subject, sky, skin, eyes and hair
masks and the depth map come along: named alpha channels in a TIFF, masked layer groups in
a PSD, DNG 1.6 semantic masks and a DNG 1.5 depth map in an enhanced (linear) DNG:

```bash
cargo run --release -p epikos-cli -- export <RAW> -o out.tif --space prophoto --masks --depth
cargo run --release -p epikos-cli -- export <RAW> -o out.psd --masks
cargo run --release -p epikos-cli -- export <RAW> -o out.dng --masks --depth
cargo run --release -p epikos-cli -- export <RAW> -o out.jpg --long-edge 2048   # 8-bit JPEG, ICC + EXIF
cargo run --release -p epikos-cli -- export <RAW> -o out.png                    # 16-bit PNG
```

Every format works for every source. An enhanced DNG made from a JPEG, PNG or TIFF holds
its decoded pixels made linear: it edits like a raw file, but has no more latitude than
the original. JPEG and PNG have no room for mask channels.

The app's Export dialog can then open the TIFF in an installed editor (Photoshop, Lightroom,
Capture One, DxO PhotoLab, Affinity Photo, Pixelmator Pro).

Scene analysis (Section 2) and story-arc grouping of a folder, as JSON:

```bash
cargo run --release -p epikos-cli -- analyze <RAW>   # genre, light, skin, palette, histogram (~3 s with models)
cargo run --release -p epikos-cli -- story <DIR>     # groups, hero frames, palettes (previews + EXIF only)
cargo run --release -p epikos-cli -- learn <PHOTO> --name "My look"   # learn a style (saved in ~/.epikos)
cargo run --release -p epikos-cli -- mentor <RAW> --target <learned-id>  # starting point aimed at it
```

Genres are scored by hand-written rules over measured cues (skin and subject coverage,
sky, depth range, focal length, light colour, capture time, straight lines, stars), not
a trained classifier; each score carries the evidence behind it. In the app, "Sync this
look" copies the open photo's look to the rest of its story-arc group, adapting exposure,
white-balance shift, skin protection and micro-contrast per frame. Each changed sidecar is
backed up to `<file>.epikos.json.presync`, so "Undo sync" can restore it.

Keep the checkout out of iCloud Drive (e.g. `~/Developer`), or mark the folder "Keep
Downloaded". With "Optimize Mac Storage", macOS evicts project files (`node_modules`,
`target`, `.git`, models) and reads of them can time out, which crashes dev servers and
builds at random. The mask and depth models are read into memory before loading, which
survives an eviction but still waits for the download.

Panics are logged with a backtrace to `~/Library/Logs/EPIKOS RAW/crash.log`; a panic in a
command becomes an error message in the app instead of closing it.

## Standalone app

For every Mac (Apple Silicon and Intel), as one universal app:

```bash
scripts/fetch-models.sh                     # the models are bundled into the app (~510 MB)
scripts/build-onnxruntime-universal.sh      # once: ONNX Runtime for arm64 + x86_64 (~15 min, ~1 GB)
scripts/build-mac-app.sh
```

This writes `target/universal-apple-darwin/release/bundle/macos/EPIKOS RAW.app` and a DMG
beside it in `bundle/dmg/`. The models, ONNX Runtime (linked statically) and the lens
database are inside the app, so it works offline with nothing else installed. It needs
macOS 13.3 or later.

- `ort` only ships a prebuilt ONNX Runtime for Apple Silicon, hence the source build:
  one build per architecture (ONNX Runtime's own two-architecture mode doesn't compile
  in 1.28), merged with `lipo`, which `build-mac-app.sh` points `ORT_LIB_LOCATION` at.
  The ONNX Runtime version must match the one `ort` expects (1.28.0 for ort 2.0.0-rc.13).
- An Apple-Silicon-only build needs none of that: `cd apps/desktop && CI=true npm run tauri build`.
- `CI=true` skips the Finder AppleScript that arranges the DMG window; without Automation
  access to Finder that step times out and the DMG isn't written.
- Not signed or notarised: on another Mac, the first launch needs right-click → Open (or
  System Settings → Privacy & Security → Open Anyway).

The face-parsing model (eyes and hair) is MIT-licensed, but it was trained on
CelebAMask-HQ, whose images are licensed for non-commercial research only. The LaMa
inpainting model is Apache-2.0 but trained on Places2, and FCN-ResNet50 on COCO / Pascal
VOC. Check those before distributing the app commercially. The film looks are inspired
by the Fujifilm and Leica renderings; they are not the manufacturers' own profiles.

App icons are placeholders; regenerate the full set with
`npm run tauri icon src-tauri/icons/app-icon-source.png`.
