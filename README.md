# EPIKOS RAW

Cross-platform RAW/DNG editor (JPEG and PNG too). Product spec: [PRD.md](PRD.md).

## Layout

| Path | What |
|---|---|
| `crates/epikos-core` | 32-bit float image types, camera formats, sensor profiles, lens-profile models |
| `crates/epikos-decode` | RAW/DNG decode via `rawler` (plus the DNG's own lens corrections from `OpcodeList3`), JPEG/PNG decode, crop, embedded thumbnails |
| `crates/epikos-pipeline` | Demosaic, highlights, white balance, noise reduction, lens profiles and optics, colour, exposure, Step 2 tone and auto-tone, straighten / vertical perspective and auto-upright, Step 3 local mask adjustments, Step 4 texture / retouching, Step 5 HSL, colour wheels, foliage shift and background re-colouration, Step 6 glow / depth fog / light shafts, Step 7 parametric and point curves, split toning, Step 8 grain and vignette, 3D virtual lights, parametric styles and style fusion (`src/look`), preview binning, display transform |
| `crates/epikos-sidecar` | Non-destructive `.epikos.json` (canonical) and Adobe-compatible `.xmp` |
| `crates/epikos-masks` | On-device models via ONNX Runtime (CPU): IS-Net subject and skyseg sky masks, BiSeNet face parsing for eyes and hair (Step 3), Depth Anything V2 Small depth (Steps 3 and 6) |
| `crates/epikos-engine` | Session layer for front-ends: image cache, previews, sidecar policy, Step 3 masks (subject, background, sky, skin, eyes, hair, foreground), the Lensfun lens database (`data/lensfun`, CC BY-SA 3.0), TIFF / layered PSD / enhanced DNG export, natural-language look prompts, scene analysis and story-arc sync |
| `crates/epikos-cli` | `epikos inspect / sidecar / develop` |
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
```

The app's Export dialog can then open the TIFF in an installed editor (Photoshop, Lightroom,
Capture One, DxO PhotoLab, Affinity Photo, Pixelmator Pro).

Scene analysis (Section 2) and story-arc grouping of a folder, as JSON:

```bash
cargo run --release -p epikos-cli -- analyze <RAW>   # genre, light, skin, palette (~3 s with models)
cargo run --release -p epikos-cli -- story <DIR>     # groups, hero frames, palettes (previews + EXIF only)
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

```bash
scripts/fetch-models.sh                # the models are bundled into the app (~510 MB)
cd apps/desktop && CI=true npm run tauri build
```

This writes `target/release/bundle/macos/EPIKOS RAW.app` and
`target/release/bundle/dmg/EPIKOS RAW_0.1.0_aarch64.dmg` (about 480 MB). The models, ONNX
Runtime (linked statically) and the lens database are inside the app, so it works offline
with nothing else installed.

- `CI=true` skips the Finder AppleScript that arranges the DMG window; without Automation
  access to Finder that step times out and the DMG isn't written.
- Apple Silicon only: `ort` has no prebuilt ONNX Runtime for Intel Macs, so an Intel or
  universal build needs ONNX Runtime compiled for `x86_64-apple-darwin` first.
- Not signed or notarised: on another Mac, the first launch needs right-click → Open (or
  System Settings → Privacy & Security → Open Anyway).

The face-parsing model (eyes and hair) is MIT-licensed, but it was trained on
CelebAMask-HQ, whose images are licensed for non-commercial research only. Check that
before distributing the app commercially.

App icons are placeholders; regenerate the full set with
`npm run tauri icon src-tauri/icons/app-icon-source.png`.
