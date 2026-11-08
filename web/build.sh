#!/bin/sh
# Builds the browser version into one self-contained file, dist/index.html.
# Needs: rustup target add wasm32-unknown-unknown
#        cargo install wasm-bindgen-cli --version <the wasm-bindgen version in Cargo.lock>
# wasm-opt (from binaryen) is used when present, to make the file smaller.
set -eu
cd "$(dirname "$0")/.."

cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --target no-modules --no-typescript --out-dir target/web \
    target/wasm32-unknown-unknown/release/orbis.wasm
if command -v wasm-opt >/dev/null; then
    wasm-opt -O2 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
        -o target/web/orbis_bg.wasm target/web/orbis_bg.wasm
fi
python3 web/pack.py target/web/orbis.js target/web/orbis_bg.wasm web/index.html dist/index.html
