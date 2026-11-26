#!/bin/bash
# Build Creek Laser Tag: the server binary into bin/ and the browser game into web/laser-tag/pkg/.
# Needs Docker; the first run builds the "lasertag-build" image (Rust + the WebAssembly target + wasm-bindgen).
set -euo pipefail
cd "$(dirname "$0")"
if ! docker image inspect lasertag-build >/dev/null 2>&1; then
  docker build -t lasertag-build -f ref/Dockerfile.build ref
fi
docker run --rm -v "$PWD":/src -v lasertag-cargo:/usr/local/cargo/registry -w /src -u "$(id -u):$(id -g)" -e CARGO_HOME=/src/target/cargo-home lasertag-build bash -c '
  set -e
  mkdir -p target/cargo-home && ln -sfn /usr/local/cargo/registry target/cargo-home/registry 2>/dev/null || true
  cargo build --release -p lasertag-server
  cargo build --release -p lasertag-client --target wasm32-unknown-unknown
  wasm-bindgen --target web --no-typescript --out-dir web/laser-tag/pkg target/wasm32-unknown-unknown/release/lasertag_client.wasm
'
mkdir -p bin
install -m 755 target/release/lasertag-server bin/lasertag-server.new
mv -f bin/lasertag-server.new bin/lasertag-server
ls -la bin/lasertag-server web/laser-tag/pkg/
