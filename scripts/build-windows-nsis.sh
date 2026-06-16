#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_DIR="/tmp/auto-judge-win-build"

rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"
rsync -a --delete --exclude node_modules --exclude src-tauri/target "$ROOT_DIR/" "$BUILD_DIR/"

cd "$BUILD_DIR"
npm install
npm run build:web
npx tauri build --bundles nsis --target x86_64-pc-windows-gnu

mkdir -p "$ROOT_DIR/src-tauri/target/x86_64-pc-windows-gnu/release/bundle/nsis"
rsync -a "$BUILD_DIR/src-tauri/target/x86_64-pc-windows-gnu/release/bundle/nsis/" \
  "$ROOT_DIR/src-tauri/target/x86_64-pc-windows-gnu/release/bundle/nsis/"
