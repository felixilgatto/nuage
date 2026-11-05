#!/usr/bin/env bash
# Tests the site made by scripts/flatpak-repo.sh as a new user would use it,
# in a throw-away Flatpak installation (FLATPAK_USER_DIR), served locally:
#   1. install from nuage.flatpakref (adds the signed repository, and the
#      GNOME runtime from Flathub), run `nuage --version`;
#   2. publish a new commit to (a copy of) the repository: `flatpak update`
#      must get it;
#   3. install the bundle instead: it must add the repository as its
#      origin, so that it updates too.
# Usage: scripts/test-flatpak-repo.sh <gpg-key-id> <site> <bundle>
# GNUPGHOME selects the keyring holding the private key (for step 2).
set -euo pipefail

key=$1 bundle=$3
app=io.github.felixilgatto.Nuage
work=$(mktemp -d)
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
url="http://127.0.0.1:$port"
cp -a "$2" "$work/site"
site=$work/site
# The site points at GitHub Pages; here it is served locally.
sed -i "s|^Url=.*|Url=$url/repo/|" "$site/nuage.flatpakref" "$site/nuage.flatpakrepo"
python3 -m http.server --bind 127.0.0.1 --directory "$site" "$port" >/dev/null 2>&1 &
server=$!
trap 'kill $server; gpgconf --kill gpg-agent 2>/dev/null; rm -rf "$work"' EXIT
export FLATPAK_USER_DIR=$work/flatpak
for _ in $(seq 50); do python3 -c "import urllib.request; urllib.request.urlopen('$url/')" 2>/dev/null && break; sleep 0.1; done

echo "== 1. install from the .flatpakref"
flatpak --user install -y --noninteractive "$url/nuage.flatpakref"
flatpak --user remotes --columns=name,url
# Signed: the remote verifies GPG signatures.
flatpak --user remote-ls --columns=ref nuage | grep -q "^app/$app/"
first=$(flatpak --user info --show-commit "$app")
flatpak run --command=nuage "$app" --version

echo "== 2. a new version reaches flatpak update"
ref=$(flatpak --user info --show-ref "$app")
flatpak build-commit-from --src-repo="$site/repo" --gpg-sign="$key" \
    --gpg-homedir="${GNUPGHOME:-$HOME/.gnupg}" --subject="Update test" --force \
    --no-update-summary "$site/repo" "$ref"
flatpak build-update-repo --gpg-sign="$key" --gpg-homedir="${GNUPGHOME:-$HOME/.gnupg}" "$site/repo"
flatpak --user update -y --noninteractive "$app"
second=$(flatpak --user info --show-commit "$app")
[[ "$first" != "$second" ]] || { echo "flatpak update did not update" >&2; exit 1; }
echo "updated: $first -> $second"

echo "== 3. the bundle updates from the repository"
flatpak --user uninstall -y --noninteractive "$app"
flatpak --user install -y --noninteractive --bundle "$bundle"
flatpak run --command=nuage "$app" --version
origin=$(flatpak --user info --show-origin "$app")
flatpak --user remotes --columns=name,url | grep "^$origin"
flatpak --user remote-modify --url="$url/repo/" "$origin"   # the bundle points at Pages
flatpak --user update -y --noninteractive "$app"
[[ "$(flatpak --user info --show-commit "$app")" == "$second" ]] || { echo "the bundle did not update" >&2; exit 1; }
echo "flatpak repository test passed"
