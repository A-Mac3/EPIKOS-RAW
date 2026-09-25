# EPIKOS RAW

Cross-platform RAW/DNG editor. Product spec: [PRD.md](PRD.md).

## Layout

| Path | What |
|---|---|
| `crates/epikos-core` | 32-bit float image types, camera formats, sensor profiles |
| `crates/epikos-decode` | RAW/DNG decode via `rawler`, crop, embedded thumbnails |
| `crates/epikos-pipeline` | Demosaic, highlights, white balance, noise reduction, optics, colour, exposure, Step 4 texture / retouching, Step 5 HSL and colour wheels, parametric styles (`src/look`), preview binning, display transform |
| `crates/epikos-sidecar` | Non-destructive `.epikos.json` (canonical) and Adobe-compatible `.xmp` |
| `crates/epikos-masks` | Step 3 AI masks: IS-Net (subject) and skyseg (sky) ONNX models on the CPU via ONNX Runtime |
| `crates/epikos-engine` | Session layer for front-ends: image cache, previews, sidecar policy |
| `crates/epikos-cli` | `epikos inspect / sidecar / develop` |
| `apps/desktop` | Tauri 2 + React/TypeScript desktop app (`src-tauri` = Rust commands) |

## Develop

Requires Rust (stable) and Node.js 20+.

```bash
cargo test --workspace                 # engine tests
scripts/fetch-models.sh                # once: ~355 MB of mask models into models/ (git-ignored)
cd apps/desktop && npm install         # once
npm run tauri dev                      # desktop app with hot reload
```

UI-only work without the engine: `npm run dev`, then open `http://localhost:5173/?mock`
(synthetic test chart, dev builds only).

Render a real file through the engine and time it:

```bash
cargo run --release -p epikos-engine --example render_preview -- <RAW> <OUT_DIR> [EV]
```

Render a file with each built-in style (plus its skin map) and time full-resolution develops:

```bash
cargo run --release -p epikos-engine --example render_looks -- <RAW> <OUT_DIR> [--full]
```

Export a full-resolution 16-bit TIFF (sRGB, Display P3 or ProPhoto, ICC embedded):

```bash
cargo run --release -p epikos-cli -- export <RAW> -o out.tif --space prophoto
```

App icons are placeholders; regenerate the full set with
`npm run tauri icon src-tauri/icons/app-icon-source.png`.
