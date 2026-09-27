#!/usr/bin/env bash
# Build the standalone macOS app for every Mac (Apple Silicon and Intel): a universal
# .app and .dmg with the models and a statically linked ONNX Runtime inside.
#
#   scripts/build-mac-app.sh [ONNXRUNTIME_WORK_DIR]   (default: ../onnxruntime-build)
#
# Run scripts/fetch-models.sh and scripts/build-onnxruntime-universal.sh first.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(cd "${1:-$REPO/../onnxruntime-build}" && pwd)"
if [[ ! -f "$WORK/universal/libonnxruntime.a" ]]; then
  echo "No universal ONNX Runtime in $WORK/universal: run scripts/build-onnxruntime-universal.sh" >&2
  exit 1
fi
for model in isnet-general-use skyseg depth-anything-v2-small face-parsing-resnet18 fcn-resnet50-12 lama-fp32; do
  if [[ ! -f "$REPO/models/$model.onnx" ]]; then
    echo "Missing models/$model.onnx: run scripts/fetch-models.sh" >&2
    exit 1
  fi
done

# ort links this ONNX Runtime (both architectures) instead of downloading its
# Apple-Silicon-only build. CI=true skips the Finder AppleScript that lays out the DMG
# window, which times out without Automation access to Finder.
export ORT_LIB_LOCATION="$WORK/universal"
export CI=true
rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
# ort-sys decides how to link when its build script runs, and cargo only reruns it when
# ORT_LIB_LOCATION changes, not the library in it: start it fresh.
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo clean -q -p ort-sys --release --target "$target" --manifest-path "$REPO/Cargo.toml"
done
cd "$REPO/apps/desktop"
npm run tauri build -- --target universal-apple-darwin

echo
echo "App: $REPO/target/universal-apple-darwin/release/bundle/macos/EPIKOS RAW.app"
ls "$REPO"/target/universal-apple-darwin/release/bundle/dmg/*.dmg
