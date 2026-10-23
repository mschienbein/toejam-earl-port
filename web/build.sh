#!/usr/bin/env bash
# Usage: WASM_BINDGEN=/path/to/wasm-bindgen CARGO_TARGET_DIR=/path/to/target bash web/build.sh
# Install the version-matched CLI with:
# cargo install wasm-bindgen-cli --version 0.2.129 --locked --root "$HOME/.local"
# Serve this directory over HTTPS or localhost after building. The static output
# uses only relative URLs and needs neither COOP/COEP nor a server-side ROM API.
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BINDGEN="${WASM_BINDGEN:-wasm-bindgen}"
if [[ "$("$BINDGEN" --version)" != 'wasm-bindgen 0.2.129' ]]; then
  printf '%s\n' 'tje-web requires wasm-bindgen CLI 0.2.129 (matching Cargo.toml).' >&2
  exit 1
fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/scratch/web-target}"
cargo --config "$ROOT/.cargo/config.toml" build --locked --manifest-path "$ROOT/Cargo.toml" --package tje-web --target wasm32-unknown-unknown --release
"$BINDGEN" --target web --out-dir "$ROOT/web/pkg" --out-name tje_web \
  "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/tje_web.wasm"
printf '%s\n' 'Built web/pkg. Serve web/ on HTTPS or localhost; no isolation headers are required.'
