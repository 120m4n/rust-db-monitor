#!/bin/sh
# Build release para macOS Apple Silicon: deja dist/db-monitor (+ config.example.toml) y borra target/.
#   ./build.sh            compila, copia a dist/ y hace cargo clean
#   ./build.sh --keep     igual pero conserva target/ (rebuilds más rápidos)
set -eu
cd "$(dirname "$0")"

# rustup de Homebrew no pone cargo/rustc en el PATH
TC="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin"
[ -d "$TC" ] && PATH="$TC:$PATH"

# el rust-objcopy del toolchain local falla (falta libLLVM): se hace strip con el del sistema
CARGO_PROFILE_RELEASE_STRIP=false cargo build --release --target aarch64-apple-darwin

rm -rf dist && mkdir dist
cp target/aarch64-apple-darwin/release/db-monitor dist/
strip dist/db-monitor
cp config.example.toml dist/

[ "${1:-}" = "--keep" ] || cargo clean

file dist/db-monitor
ls -lh dist
