#!/usr/bin/env bash
# Puts the homepage (website/) into the site published on GitHub Pages,
# next to the Flatpak repository, with the site's address filled in.
# Usage: scripts/site-page.sh <base-url> <site>
# Without arguments: a local preview in target/site, to open in a browser.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
if [ $# -eq 0 ]; then
    set -- "file://$root/target/site" "$root/target/site"
    echo "preview: open $root/target/site/index.html"
elif [ $# -ne 2 ]; then
    echo "usage: $0 [<base-url> <site>]" >&2
    exit 2
fi
base=${1%/} site=$2
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
