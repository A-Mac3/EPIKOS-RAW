#!/usr/bin/env bash
# Download the ONNX models used for Step 3 (AI Subject & Semantic Masking: subject, sky,
# face features, people, vehicles, animals), Step 6 (depth-based fog) and generative erase and verify their SHA-256. Models are not committed; the engine
# looks for them in $EPIKOS_MODELS_DIR, then the app data folder, then <repo>/models.
#
#   scripts/fetch-models.sh [DEST_DIR]      (default: <repo>/models)
set -euo pipefail

DEST="${1:-$(cd "$(dirname "$0")/.." && pwd)/models}"
mkdir -p "$DEST"

# name | url | sha256
MODELS=(
  "isnet-general-use.onnx|https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx|60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a"
  "skyseg.onnx|https://huggingface.co/JianyuanWang/skyseg/resolve/main/skyseg.onnx|ab9c34c64c3d821220a2886a4a06da4642ffa14d5b30e8d5339056a089aa1d39"
  # Depth Anything V2 Small (Apache-2.0; the Base/Large weights are non-commercial).
  "depth-anything-v2-small.onnx|https://huggingface.co/onnx-community/depth-anything-v2-small/resolve/main/onnx/model.onnx|afb6a5c28f3b6bf1618c6e43f02073ef9dfdc70e937502d51603e57b0a1df10c"
  # BiSeNet face parsing (code and weights MIT, yakhyo/face-parsing), for the eye and
  # hair masks. Trained on CelebAMask-HQ, whose images are for non-commercial research.
  "face-parsing-resnet18.onnx|https://github.com/yakhyo/face-parsing/releases/download/weights/resnet18.onnx|0d9bd318e46987c3bdbfacae9e2c0f461cae1c6ac6ea6d43bbe541a91727e33f"
  # FCN-ResNet50 semantic segmentation from the ONNX Model Zoo (Apache-2.0; torchvision
  # weights, BSD-3): people, vehicles and animals (the 21 Pascal VOC classes).
  "fcn-resnet50-12.onnx|https://github.com/onnx/models/raw/main/validated/vision/object_detection_segmentation/fcn/model/fcn-resnet50-12.onnx|eb5017d1b80372eb0b58552655274698817ba2e774437e2c2d3c0a613d2e99bd"
  # LaMa inpainting (big-lama, Apache-2.0; ONNX port by Carve) for generative erase.
  # Trained on Places2, whose images are for non-commercial research.
  "lama-fp32.onnx|https://huggingface.co/Carve/LaMa-ONNX/resolve/main/lama_fp32.onnx|1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6"
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
