#!/usr/bin/env bash
# Enforces the dependency rules between Nuage crates (see docs/ARCHITECTURE.md):
#
#   1. each workspace crate depends only on the project crates it is allowed to;
#   2. `nuaged` and `nuage` (the headless binaries) pull no GTK crate at all.
#
# Needs: cargo, jq. Does not compile anything, so it runs without GTK headers.
set -euo pipefail

cd "$(dirname "$0")/.."

command -v jq >/dev/null || { echo "check-deps: jq is required" >&2; exit 2; }

# Allowed project dependencies, per package (any dependency kind).
declare -A ALLOWED=(
    [nuage-core]=""
    [nuage-ipc]="nuage-core"
    [nuage-providers]="nuage-core"
    [nuage-rclone]="nuage-core"
    [nuage-fs]="nuage-core"
    [nuage-engine]="nuage-core nuage-rclone nuage-providers nuage-fs"
    [nuaged]="nuage-engine nuage-ipc nuage-core"
    [nuage]="nuage-core nuage-ipc"
    [nuage-ui]="nuage-core nuage-ipc"
    [nuage-tray]="nuage-core nuage-ipc"
    [nuage-gtk]="nuage-core nuage-ipc nuage-ui nuage-tray"
)

# Crates that must never appear in the dependency tree of a headless binary.
GTK_CRATES_RE='^(gtk4|gtk4-sys|gdk4|gdk4-sys|gsk4|gsk4-sys|libadwaita|libadwaita-sys|gdk-pixbuf|gdk-pixbuf-sys|pango|pango-sys|cairo-rs|cairo-sys-rs|graphene-rs|graphene-sys|glib|glib-sys|gio|gio-sys|gobject-sys|ksni|nuage-ui|nuage-tray|nuage-gtk)$'

status=0
metadata=$(cargo metadata --format-version 1 --no-deps)
members=$(jq -r '.packages[].name' <<<"$metadata")

for pkg in $members; do
    if [[ ! -v "ALLOWED[$pkg]" ]]; then
        echo "check-deps: $pkg is not listed in scripts/check-deps.sh; add its rule" >&2
        status=1
        continue
    fi
    # Project dependencies = dependencies whose name is a workspace member.
    deps=$(jq -r --arg pkg "$pkg" --argjson members "$(jq '[.packages[].name]' <<<"$metadata")" '
        .packages[] | select(.name == $pkg) | .dependencies[]
        | select(.name as $n | $members | index($n)) | .name' <<<"$metadata" | sort -u)
    for dep in $deps; do
        if ! grep -qw -- "$dep" <<<"${ALLOWED[$pkg]}"; then
            echo "check-deps: $pkg must not depend on $dep (allowed: ${ALLOWED[$pkg]:-none})" >&2
            status=1
        fi
    done
done

for bin_pkg in nuaged nuage; do
    tree=$(cargo tree -p "$bin_pkg" --edges normal,build --prefix none --format '{p}' --target all)
    offenders=$(awk '{print $1}' <<<"$tree" | sort -u | grep -E "$GTK_CRATES_RE" || true)
    if [[ -n "$offenders" ]]; then
        echo "check-deps: $bin_pkg pulls desktop/GTK crates:" $offenders >&2
        status=1
    fi
done

if [[ $status -eq 0 ]]; then
    echo "check-deps: OK"
fi
exit $status
