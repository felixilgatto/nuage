#!/usr/bin/env bash
# Builds the headless packages for this machine's architecture (x86_64 or
# aarch64) into target/dist/:
#   nuage-<version>-<arch>-linux.tar.gz, nuage_<version>_<amd64|arm64>.deb,
#   nuage-<version>-1.<arch>.rpm
# Static binaries (musl, no C dependency), with the pinned rclone. Needs the
# <arch>-unknown-linux-musl Rust target, musl-tools, cargo-deb,
# cargo-generate-rpm and jq.
set -euo pipefail

cd "$(dirname "$0")/.."
arch=$(uname -m)
TARGET=$arch-unknown-linux-musl
version=$(cargo metadata --format-version 1 --no-deps | jq -r '.packages[] | select(.name == "nuaged") | .version')


scripts/fetch-rclone.sh
cargo build --release --locked --target "$TARGET" -p nuaged -p nuage
rel="target/$TARGET/release"

dist=target/dist
name="nuage-$version-$arch-linux"
rm -rf "$dist/$name"
mkdir -p "$dist/$name"
cp "$rel/nuaged" "$rel/nuage" target/rclone/rclone build-aux/native/install.sh \
    build-aux/native/nuaged.service LICENSE README.md \
    docs/SERVER.md docs/PROVIDERS.md docs/GOOGLE_OAUTH.md docs/CLI.md docs/TROUBLESHOOTING.md "$dist/$name/"
cp target/rclone/COPYING "$dist/$name/rclone-COPYING"
tar -C "$dist" -czf "$dist/$name.tar.gz" "$name"
rm -rf "${dist:?}/$name"

cargo deb --no-build --target "$TARGET" -p nuaged --output "$dist/"
cargo generate-rpm --target "$TARGET" -p crates/daemon --output "$dist/"

(cd "$dist" && sha256sum ./*.tar.gz ./*.deb ./*.rpm > SHA256SUMS)
ls -l "$dist"
