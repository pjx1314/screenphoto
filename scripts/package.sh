#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
out="${1:-"$root/dist"}"
target="${2:-}"
build_dir="${CARGO_TARGET_DIR:-"$root/target"}"
if [[ "$build_dir" != /* ]]; then build_dir="$PWD/$build_dir"; fi
target_args=()
binary="$build_dir/release/screenphoto"
if [[ -n "$target" ]]; then
  target_args=(--target "$target")
  binary="$build_dir/$target/release/screenphoto"
fi
if [[ "$target" == *windows* || ( -z "$target" && "${OS:-}" == Windows_NT ) ]]; then binary+=".exe"; fi

cargo --config "$root/.cargo/config.toml" build --manifest-path "$root/Cargo.toml" --release --locked "${target_args[@]}" --bin screenphoto
exec cargo --config "$root/.cargo/config.toml" run --manifest-path "$root/Cargo.toml" --release --locked --features dev-tools --bin screenphoto-dev -- --root "$root" package --binary "$binary" "${target_args[@]}" --out "$out"
