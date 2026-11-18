#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
if [[ $(uname -s) != Darwin ]]; then
  echo "Build the Dockon DMG on macOS." >&2
  exit 1
fi

target=$(rustc -vV | sed -n 's/^host: //p')
case "$target" in
  aarch64-apple-darwin|x86_64-apple-darwin) ;;
  *) echo "Unsupported macOS target: $target" >&2; exit 1 ;;
esac

stage=.local/macos-installer
mkdir -p "$stage"
cargo build --locked --release --target "$target" \
  -p dockon-daemon --bin dockon-daemon --bin dockon-runner --bin dockonctl \
  -p dockon-router --bin dockon-hosts-helper
for binary in dockon-daemon dockon-runner dockonctl dockon-hosts-helper; do
  install -m 755 "target/$target/release/$binary" "$stage/$binary-$target"
done

pnpm --filter @dockon/desktop tauri build --ci --target "$target" \
  --config src-tauri/tauri.macos-installer.json -- --locked
