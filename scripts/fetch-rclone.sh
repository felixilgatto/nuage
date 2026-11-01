#!/usr/bin/env bash
# Downloads the rclone version bundled with Nuage into target/rclone/, checks
# its sha256, and fetches its license. Keep the version and checksum in sync
# with the Flatpak manifest.
set -euo pipefail

VERSION=v1.75.2
SHA256=349ac8fba6ff65d6247043f1750cdcb518ec5d500ef91463a10d37c0ccdf3702

cd "$(dirname "$0")/.."
out=target/rclone
mkdir -p "$out"
if [[ -x "$out/rclone" ]] && "$out/rclone" version 2>/dev/null | head -1 | grep -q " $VERSION\$"; then
    echo "rclone $VERSION already in $out"
    exit 0
fi

zip="rclone-$VERSION-linux-amd64.zip"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/$zip" "https://downloads.rclone.org/$VERSION/$zip"
echo "$SHA256  $tmp/$zip" | sha256sum -c --quiet -
unzip -q -o -j "$tmp/$zip" "rclone-$VERSION-linux-amd64/rclone" -d "$out"
chmod 755 "$out/rclone"
curl -fsSL -o "$out/COPYING" "https://raw.githubusercontent.com/rclone/rclone/$VERSION/COPYING"
echo "rclone $VERSION in $out"
