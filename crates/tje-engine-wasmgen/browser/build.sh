#!/usr/bin/env bash
set -euo pipefail
ROOT="$(realpath "$(dirname "$0")/../../..")"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/scratch/wasm-proof/target}"
OUT="${TJE_WASM_PROOF_DIR:-$ROOT/scratch/wasm-proof/pkg}"
mkdir -p "$OUT"
cargo --config "$ROOT/.cargo/config.toml" build --manifest-path "$ROOT/Cargo.toml" --locked --release --lib \
  -p tje-engine-wasmgen --features browser-proof --target wasm32-unknown-unknown
wasm-bindgen --target web --out-name engine_proof --out-dir "$OUT" \
  "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/tje_engine_wasmgen.wasm"
printf '%s\n' "Browser proof package: $OUT (development oracle, never distribute)."
