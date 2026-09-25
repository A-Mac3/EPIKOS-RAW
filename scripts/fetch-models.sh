#!/usr/bin/env bash
# Download the ONNX models used for Step 3 (AI Subject & Semantic Masking) and verify
# their SHA-256. Models are not committed; the engine looks for them in
# $EPIKOS_MODELS_DIR, then the app data folder, then <repo>/models.
#
#   scripts/fetch-models.sh [DEST_DIR]      (default: <repo>/models)
set -euo pipefail

DEST="${1:-$(cd "$(dirname "$0")/.." && pwd)/models}"
mkdir -p "$DEST"

# name | url | sha256
MODELS=(
  "isnet-general-use.onnx|https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx|60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a"
  "skyseg.onnx|https://huggingface.co/JianyuanWang/skyseg/resolve/main/skyseg.onnx|ab9c34c64c3d821220a2886a4a06da4642ffa14d5b30e8d5339056a089aa1d39"
)

sha256() { shasum -a 256 "$1" 2>/dev/null | cut -d' ' -f1 || sha256sum "$1" | cut -d' ' -f1; }

for entry in "${MODELS[@]}"; do
  IFS='|' read -r name url want <<<"$entry"
  out="$DEST/$name"
  if [[ -f "$out" && "$(sha256 "$out")" == "$want" ]]; then
    echo "✓ $name (already present)"
    continue
  fi
  echo "↓ $name"
  curl -fL --retry 3 --progress-bar -o "$out.partial" "$url"
  got="$(sha256 "$out.partial")"
  if [[ "$got" != "$want" ]]; then
    rm -f "$out.partial"
    echo "✗ $name: SHA-256 mismatch (got $got, want $want)" >&2
    exit 1
  fi
  mv "$out.partial" "$out"
  echo "✓ $name"
done
echo "Models in $DEST"
