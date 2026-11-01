#!/usr/bin/env bash
# A demo setup for screenshots, apart from your own configuration: its own
# home, daemon and socket, a "local" account named "Documents" filled with
# sample files, an item kept offline and an excluded folder.
#
#   scripts/demo.sh            # sets it up, prints how to start the app on it
#   scripts/demo.sh stop       # stops its daemon and deletes it
#
# Needs target/release/{nuaged,nuage} (cargo build --release -p nuaged -p nuage),
# NUAGE_RCLONE (or rclone in PATH) and the desktop app (target/.../nuage-gtk).
set -euo pipefail

cd "$(dirname "$0")/.."
demo=${NUAGE_DEMO:-/tmp/nuage-demo}
bin=target/release
export HOME="$demo/home"
export XDG_CONFIG_HOME="$HOME/.config" XDG_CACHE_HOME="$HOME/.cache" XDG_DATA_HOME="$HOME/.local/share"
export NUAGE_SOCKET="$demo/nuaged.sock" NUAGE_KEYRING=off

if [ "${1:-}" = stop ]; then
    "$bin/nuage" daemon stop --now >/dev/null 2>&1 || true
    sleep 2
    rm -rf "$demo"
    echo "Demo removed."
    exit 0
fi

mkdir -p "$HOME" "$demo/cloud"
src="$demo/cloud"
mkdir -p "$src"/{Projects/Website,Projects/Garden,Photos/Holidays,Passwords,Archives/2019,Recipes}
printf 'Website redesign\n' >"$src/Projects/Website/plan.md"
printf 'Spring planting\n' >"$src/Projects/Garden/calendar.ods"
head -c 2400000 /dev/urandom >"$src/Photos/Holidays/beach.jpg"
head -c 1800000 /dev/urandom >"$src/Photos/Holidays/mountains.jpg"
head -c 64000 /dev/urandom >"$src/Passwords/vault.kdbx"
head -c 9000000 /dev/urandom >"$src/Archives/2019/backup.zip"
printf 'Flour, butter, sugar\n' >"$src/Recipes/shortbread.txt"
head -c 380000 /dev/urandom >"$src/Report 2026.pdf"
printf 'Remember the milk\n' >"$src/Notes.txt"

"$bin/nuaged" >"$demo/nuaged.log" 2>&1 &
for _ in $(seq 50); do "$bin/nuage" daemon status >/dev/null 2>&1 && break; sleep 0.2; done
"$bin/nuage" account add local --name Documents --param "root=$src" \
    --mount-point "$HOME/Cloud/Documents" >/dev/null
"$bin/nuage" pin add "Documents:Passwords" >/dev/null
"$bin/nuage" exclude add Documents Archives >/dev/null
"$bin/nuage" status

cat <<EOF

Demo ready. Close Nuage first (same application ID), then start it on the demo,
in the light style:

  HOME=$HOME XDG_CONFIG_HOME=$XDG_CONFIG_HOME XDG_CACHE_HOME=$XDG_CACHE_HOME \\
  XDG_DATA_HOME=$XDG_DATA_HOME NUAGE_SOCKET=$NUAGE_SOCKET \\
  ADW_DEBUG_COLOR_SCHEME=prefer-light target/flatpak-sdk/debug/nuage-gtk

Remove it with: scripts/demo.sh stop
EOF
