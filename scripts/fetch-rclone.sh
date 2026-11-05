#!/usr/bin/env bash
# Downloads the rclone version bundled with Nuage into target/rclone/, for
# this machine's architecture (x86_64 or aarch64), checks its sha256, and
# fetches its license. Keep the version and checksums in sync with the
# Flatpak manifest.
set -euo pipefail

VERSION=v1.75.2
case "${NUAGE_ARCH:-$(uname -m)}" in
    x86_64) ARCH=amd64 SHA256=349ac8fba6ff65d6247043f1750cdcb518ec5d500ef91463a10d37c0ccdf3702 ;;
    aarch64) ARCH=arm64 SHA256=7e1e8d69654941b7b7df84ee74c5f7fb09cce7d5496947fb7bf16b55ff427d10 ;;
    *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

cd "$(dirname "$0")/.."
out=target/rclone
mkdir -p "$out"
if [[ -x "$out/rclone" ]] && "$out/rclone" version 2>/dev/null | grep -q "^- os/arch: linux/$ARCH" \
    && "$out/rclone" version | head -1 | grep -q " $VERSION\$"; then
    echo "rclone $VERSION already in $out"
    exit 0
fi

zip="rclone-$VERSION-linux-$ARCH.zip"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/$zip" "https://downloads.rclone.org/$VERSION/$zip"
echo "$SHA256  $tmp/$zip" | sha256sum -c --quiet -
unzip -q -o -j "$tmp/$zip" "rclone-$VERSION-linux-$ARCH/rclone" -d "$out"
chmod 755 "$out/rclone"
curl -fsSL -o "$out/COPYING" "https://raw.githubusercontent.com/rclone/rclone/$VERSION/COPYING"
echo "rclone $VERSION in $out"
