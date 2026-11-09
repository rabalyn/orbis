#!/bin/sh
# Builds the browser version into one self-contained file: dist/index.html, or the path given.
# Usage: web/build.sh [output.html]   (CARGO_TARGET_DIR is honoured)
# Needs: rustup target add wasm32-unknown-unknown
#        cargo install wasm-bindgen-cli --version <the wasm-bindgen version in Cargo.lock>
# wasm-opt (from binaryen) is used when present, to make the file smaller.
set -eu
cd "$(dirname "$0")/.."
out=${1:-dist/index.html}
t=${CARGO_TARGET_DIR:-target}

cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --target no-modules --no-typescript --out-dir "$t/web" \
    "$t/wasm32-unknown-unknown/release/orbis.wasm"
if command -v wasm-opt >/dev/null; then
    wasm-opt -O2 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
        -o "$t/web/orbis_bg.wasm" "$t/web/orbis_bg.wasm"
fi
mkdir -p "$(dirname "$out")"
python3 web/pack.py "$t/web/orbis.js" "$t/web/orbis_bg.wasm" web/index.html "$out"
