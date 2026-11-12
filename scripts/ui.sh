#!/usr/bin/env bash
# Builds the UI without trunk: cargo → wasm-bindgen → app/ui/dist.
#   scripts/ui.sh build [--release]   build dist for the app
#   scripts/ui.sh serve [--release]   build with the demo data and serve on http://127.0.0.1:1420
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CMD="${1:-build}"
PROFILE=debug
CARGO_FLAGS=()
if [[ "${2:-}" == "--release" ]]; then
  PROFILE=release
  CARGO_FLAGS=(--release)
fi
# Only the browser demo carries the demo data.
if [[ "$CMD" == "serve" ]]; then
  CARGO_FLAGS+=(--features demo)
fi

DIST="$ROOT/app/ui/dist"

build() {
  cargo build --manifest-path "$ROOT/Cargo.toml" -p mostik-ui --target wasm32-unknown-unknown "${CARGO_FLAGS[@]}"
  rm -rf "$DIST"
  mkdir -p "$DIST"
  wasm-bindgen --target web --no-typescript --out-dir "$DIST" --out-name mostik_ui \
    "$ROOT/target/wasm32-unknown-unknown/$PROFILE/mostik-ui.wasm"
  cp "$ROOT/app/ui/index.html" "$DIST/"
  cp -r "$ROOT/app/ui/assets/." "$DIST/"
}

case "$CMD" in
  build) build ;;
  serve)
    build
    exec python3 -m http.server 1420 --bind 127.0.0.1 --directory "$DIST"
    ;;
  *)
    echo "usage: $0 build|serve [--release]" >&2
    exit 2
    ;;
esac
