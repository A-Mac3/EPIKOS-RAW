#!/usr/bin/env bash
# Build ONNX Runtime as universal (arm64 + x86_64) static libraries for the macOS app.
#
# `ort` downloads a prebuilt ONNX Runtime for Apple Silicon only, so a build that also
# runs on Intel Macs links against this one instead (see scripts/build-mac-app.sh).
# Both architectures come from one build: Tauri compiles the two halves of a universal
# app in one run with the same environment, so one library has to serve both.
#
#   scripts/build-onnxruntime-universal.sh [WORK_DIR]   (default: ../onnxruntime-build)
#
# Needs git, Xcode command-line tools and ~2 GB of disk; takes about 15 minutes on an
# Apple Silicon Mac. cmake and
# ninja are installed into a private venv inside WORK_DIR.
set -euo pipefail

# Must match the ONNX Runtime version `ort` downloads (ort-sys build/download/dist.tsv).
ORT_VERSION="1.28.0"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${1:-$REPO/../onnxruntime-build}"
mkdir -p "$WORK"
WORK="$(cd "$WORK" && pwd)"
cd "$WORK"

if [[ ! -x venv/bin/cmake ]]; then
  /usr/bin/python3 -m venv venv
  venv/bin/pip install -q --upgrade pip
  venv/bin/pip install -q cmake ninja
fi
export PATH="$WORK/venv/bin:$PATH"

if [[ ! -d src ]]; then
  git clone --depth 1 --branch "v$ORT_VERSION" https://github.com/microsoft/onnxruntime.git src
fi

# Static libraries only, no tests or Python. One build per architecture, merged below
# (ONNX Runtime's own two-architecture mode puts x86 compiler flags on a source file the
# arm64 kernels share, so it doesn't build). On a Mac host ONNX Runtime uses a prebuilt universal protoc, so the x86_64 build runs
# no x86_64 tools and needs no Rosetta.
for arch in arm64 x86_64; do
  cmake -S src/cmake -B "build-$arch" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_OSX_ARCHITECTURES="$arch" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=13.3 \
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
    -DPython_EXECUTABLE=/usr/bin/python3 \
    -Donnxruntime_BUILD_SHARED_LIB=OFF \
    -Donnxruntime_BUILD_UNIT_TESTS=OFF \
    -Donnxruntime_RUN_ONNX_TESTS=OFF \
    -Donnxruntime_ENABLE_PYTHON=OFF \
    -Donnxruntime_BUILD_BENCHMARKS=OFF \
    -Donnxruntime_USE_XNNPACK=OFF \
    -Donnxruntime_USE_COREML=OFF \
    -Donnxruntime_ENABLE_LTO=OFF
  ninja -C "build-$arch"
done

# One archive holding ONNX Runtime and every dependency, per architecture, then both
# architectures in one fat file: ort links a single `libonnxruntime.a` as is (its
# per-library list predates 1.28's model_package library).
rm -rf universal
mkdir -p universal
for arch in arm64 x86_64; do
  find "build-$arch" -name '*.a' -not -path '*-subbuild/*' -print0 \
    | xargs -0 libtool -static -no_warning_for_no_symbols -o "universal/onnxruntime-$arch.a" 2>/dev/null
done
lipo -create universal/onnxruntime-arm64.a universal/onnxruntime-x86_64.a -output universal/libonnxruntime.a
rm universal/onnxruntime-arm64.a universal/onnxruntime-x86_64.a

echo "ONNX Runtime $ORT_VERSION (arm64 + x86_64) in $WORK/universal"
