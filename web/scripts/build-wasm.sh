#!/usr/bin/env bash
# Build the talk WASM module and generate the JS bindings the web app imports.
#
# Output lands in web/src/wasm/ (gitignored — a build artifact). The wasm-bindgen
# CLI version MUST match the wasm-bindgen library pinned in
# crates/talk-wasm/Cargo.toml, or the generated glue mismatches the wasm ABI;
# that single pin is the source of truth and this script enforces it.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/web/src/wasm"

# The one pinned version (read from Cargo.toml so there is a single source).
PINNED="$(sed -nE 's/^wasm-bindgen = "=?([0-9.]+)".*/\1/p' \
  "$ROOT/crates/talk-wasm/Cargo.toml" | head -1)"
if [ -z "$PINNED" ]; then
  echo "could not read the pinned wasm-bindgen version from Cargo.toml" >&2
  exit 1
fi

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "wasm-bindgen CLI not found. Install the pinned version:" >&2
  echo "  cargo binstall wasm-bindgen-cli@$PINNED   # or: cargo install --version $PINNED wasm-bindgen-cli" >&2
  exit 1
fi

INSTALLED="$(wasm-bindgen --version | awk '{print $2}')"
if [ "$INSTALLED" != "$PINNED" ]; then
  echo "wasm-bindgen CLI $INSTALLED does not match the pinned library $PINNED" >&2
  echo "  cargo binstall wasm-bindgen-cli@$PINNED" >&2
  exit 1
fi

# panic=abort via RUSTFLAGS: member-crate [profile] tables are ignored by Cargo
# (profiles are read only from the workspace root), so this is the correct knob.
RUSTFLAGS="-C panic=abort" cargo build --release \
  --target wasm32-unknown-unknown -p talk-wasm

wasm-bindgen --target web --out-dir "$OUT" \
  "$ROOT/target/wasm32-unknown-unknown/release/talk_wasm.wasm"

# Shrink with binaryen when available (recommended; optional locally).
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Oz "$OUT/talk_wasm_bg.wasm" -o "$OUT/talk_wasm_bg.wasm"
  echo "wasm-opt -Oz applied"
else
  echo "note: wasm-opt not found — skipping size optimization (install binaryen)" >&2
fi

echo "wasm bindings written to $OUT"
