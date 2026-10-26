#!/usr/bin/env bash
# Private observation build. The release script never invokes this or enables
# replay-observer. Neither build includes a browser CPU oracle.
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BINDGEN="${WASM_BINDGEN:-wasm-bindgen}"
OUT="${TJE_WEB_REPLAY_DIR:?Set TJE_WEB_REPLAY_DIR to a private directory under scratch/}"
if [[ "$OUT" != /*/scratch/* ]]; then
  printf '%s\n' 'Replay packages must stay in an absolute scratch/ directory.' >&2
  exit 1
fi
if [[ "$("$BINDGEN" --version)" != 'wasm-bindgen 0.2.129' ]]; then
  printf '%s\n' 'tje-web requires wasm-bindgen CLI 0.2.129.' >&2
  exit 1
fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/scratch/web-target}"
FEATURES="replay-observer"
if [[ "${TJE_COOP_ACCEPTANCE:-0}" != 0 ]]; then
  printf '%s\n' 'The mode-owned gameplay factory is not admitted; no co-op observation package is available.' >&2
  exit 1
elif [[ "${TJE_ENHANCED:-0}" == 1 ]]; then
  FEATURES="$FEATURES,enhanced-mode"
fi
node "$ROOT/web/check-dependencies.mjs"
cargo --config "$ROOT/.cargo/config.toml" build --locked --no-default-features --features "$FEATURES" \
  --manifest-path "$ROOT/Cargo.toml" --package tje-web --target wasm32-unknown-unknown --release
"$BINDGEN" --target web --out-dir "$OUT" --out-name tje_web \
  "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/tje_web.wasm"
printf '%s\n' 'Built observation-only package in scratch; it is not a release artifact.'
