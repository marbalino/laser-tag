#!/bin/bash
# Build Creek Laser Tag in this checkout: the server into bin/ and the browser game into web/laser-tag/pkg/.
# Also runs the tests unless --no-test is given. Needs Docker; the first run builds the "lasertag-build" image
# (Rust with the WebAssembly target). The wasm-bindgen tool is installed into target/tools/, matching the
# wasm-bindgen version in Cargo.lock, so a dependency update cannot leave the two out of step.
set -euo pipefail
cd "$(dirname "$0")"
TEST=1
[ "${1:-}" = "--no-test" ] && TEST=0
if ! docker image inspect lasertag-build >/dev/null 2>&1; then
  docker build -t lasertag-build -f ref/Dockerfile.build ref
fi
docker run --rm -v "$PWD":/src -v lasertag-cargo:/usr/local/cargo/registry -w /src -e TEST=$TEST lasertag-build bash -c '
  set -euo pipefail
  if [ "$TEST" = 1 ]; then cargo test -q -p lasertag-shared -p lasertag-server; fi
  cargo build --release -p lasertag-server
  cargo build --release -p lasertag-client --target wasm32-unknown-unknown
  V=$(grep -A1 "^name = \"wasm-bindgen\"$" Cargo.lock | sed -n "s/^version = \"\(.*\)\"/\1/p")
  T=target/tools/wasm-bindgen-$V
  if [ ! -x $T/bin/wasm-bindgen ]; then cargo install -q wasm-bindgen-cli --version $V --locked --root $T; fi
  rm -rf target/pkg && $T/bin/wasm-bindgen --target web --no-typescript --out-dir target/pkg target/wasm32-unknown-unknown/release/lasertag_client.wasm
'
# put the results in place only now that everything built
mkdir -p bin web/laser-tag/pkg
install -m 755 target/release/lasertag-server bin/lasertag-server.new && mv -f bin/lasertag-server.new bin/lasertag-server
cp target/pkg/* web/laser-tag/pkg/
echo "built: bin/lasertag-server, web/laser-tag/pkg/"
