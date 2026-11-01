#!/bin/sh
# Runs flatpak-builder-lint on the manifest, the metainfo and the built repo
# (default: ./repo), and fails on any finding except the ones Nuage accepts
# for v1 (documented in docs/BUILDING.md):
#   finish-args-flatpak-spawn-access     the fusermount3 wrapper
#   finish-args-home-filesystem-access   mount folders chosen under $HOME
#   appstream-screenshots-not-mirrored-in-ostree   Flathub mirrors them
#                                                   at its own build
# Usage: scripts/flatpak-lint.sh [repo] (LINT="flatpak run --command=flatpak-builder-lint org.flatpak.Builder" locally)
set -eu
repo=${1:-repo}
lint=${LINT:-flatpak-builder-lint}
manifest=build-aux/flatpak/io.github.felixilgatto.Nuage.yml
metainfo=data/io.github.felixilgatto.Nuage.metainfo.xml

# Screenshots are published once merged into main: before that, an image
# that cannot be reached must at least be in data/screenshots/.
if ! appstream_out=$($lint appstream "$metainfo" 2>&1); then
    printf '%s\n' "$appstream_out"
    if printf '%s' "$appstream_out" | grep -E '^[EW]:' | grep -vq 'screenshot-image-not-found'; then
        echo "flatpak-builder-lint appstream: errors" >&2
        exit 1
    fi
    for url in $(grep -o 'https://[^<]*/data/screenshots/[^<]*' "$metainfo"); do
        [ -f "data/screenshots/${url##*/}" ] || { echo "missing screenshot ${url##*/}" >&2; exit 1; }
    done
    echo "Screenshots not published yet (main), present in data/screenshots/: accepted."
fi

status=0
check() {
    out=$($lint "$@" 2>&1) || true
    printf '%s\n' "$out"
    printf '%s' "$out" | python3 -c '
import json, sys
tolerated = {
    "finish-args-flatpak-spawn-access",
    "finish-args-home-filesystem-access",
    "appstream-screenshots-not-mirrored-in-ostree",
}
text = sys.stdin.read().strip()
report = json.loads(text) if text else {}
left = sorted(set(report.get("errors", [])) - tolerated)
if left:
    sys.exit("flatpak-builder-lint errors: " + ", ".join(left))
' || status=1
}
check manifest "$manifest"
check repo "$repo"
exit $status
