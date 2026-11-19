#!/bin/sh
# CI only. In the flatpak-github-actions containers, updating a runtime the
# image preinstalled can fail with "Invalid cross-device link"
# (flatpak/flatpak-github-actions#267): its folders sit in a lower layer of
# the image, and overlayfs refuses to rename them. Copy each installation to
# the job's temporary folder, a plain volume, and point flatpak at the copy.
set -eu

user_dir="${XDG_DATA_HOME:-$HOME/.local/share}/flatpak"
if [ -d "$user_dir/repo" ]; then
    cp -a "$user_dir" "$RUNNER_TEMP/flatpak-user"
    echo "FLATPAK_USER_DIR=$RUNNER_TEMP/flatpak-user" >>"$GITHUB_ENV"
fi
if [ -d /var/lib/flatpak/repo ]; then
    cp -a /var/lib/flatpak "$RUNNER_TEMP/flatpak-system"
    echo "FLATPAK_SYSTEM_DIR=$RUNNER_TEMP/flatpak-system" >>"$GITHUB_ENV"
fi
du -sh "$RUNNER_TEMP"/flatpak-* 2>/dev/null || true
