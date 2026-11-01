#!/usr/bin/env bash
# Builds the headless packages into target/dist/:
#   nuage-<version>-x86_64-linux.tar.gz and nuage_<version>_amd64.deb
# Static binaries (musl, no C dependency), with the pinned rclone. Needs the
# x86_64-unknown-linux-musl Rust target, cargo-deb and jq.
set -euo pipefail

cd "$(dirname "$0")/.."
TARGET=x86_64-unknown-linux-musl
version=$(cargo metadata --format-version 1 --no-deps | jq -r '.packages[] | select(.name == "nuaged") | .version')


scripts/fetch-rclone.sh
cargo build --release --locked --target "$TARGET" -p nuaged -p nuage
rel="target/$TARGET/release"

dist=target/dist
name="nuage-$version-x86_64-linux"
rm -rf "$dist/$name"
mkdir -p "$dist/$name"
cp "$rel/nuaged" "$rel/nuage" target/rclone/rclone build-aux/native/install.sh \
    build-aux/native/nuaged.service LICENSE README.md "$dist/$name/"
cp target/rclone/COPYING "$dist/$name/rclone-COPYING"
tar -C "$dist" -czf "$dist/$name.tar.gz" "$name"
rm -rf "${dist:?}/$name"

cargo deb --no-build --target "$TARGET" -p nuaged --output "$dist/"

(cd "$dist" && sha256sum ./*.tar.gz ./*.deb > SHA256SUMS)
ls -l "$dist"
