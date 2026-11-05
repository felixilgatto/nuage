#!/usr/bin/env bash
# Puts the homepage (website/) into the site published on GitHub Pages,
# next to the Flatpak repository, with the site's address filled in.
# Usage: scripts/site-page.sh <base-url> <site>
set -euo pipefail

base=${1%/} site=$2
root=$(dirname "$0")/..
mkdir -p "$site"
sed "s|@BASE@|$base|g" "$root/website/index.html" > "$site/index.html"
cp "$root/images/nuage-mark.svg" "$root/images/social-preview.png" \
   "$root/data/screenshots/browser.png" "$site/"
# Every local file the page links to is there (a local preview has no
# Flatpak repository: nothing to check).
[ -d "$site/repo" ] || exit 0
for f in $(grep -oE '(src|href)="[^":#]+"' "$site/index.html" | sed -E 's/^[a-z]+="//; s/"$//'); do
    [ -e "$site/$f" ] || { echo "site-page: $f is missing from $site" >&2; exit 1; }
done
