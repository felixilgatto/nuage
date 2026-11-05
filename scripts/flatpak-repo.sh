#!/usr/bin/env bash
# Makes the Flatpak repository published on GitHub Pages, from the OSTree
# repositories flatpak-builder made (one per architecture):
#   <site>/repo/             the signed repository (`flatpak update` reads it)
#   <site>/nuage.flatpakref  one-click install, also adds the repository
#   <site>/nuage.flatpakrepo the repository alone
#   <site>/index.html        the homepage, from website/ (scripts/site-page.sh)
#   <bundles>/nuage-<arch>.flatpak  bundles that update from the repository
#   <bundles>/nuage-site.tar  all of <site>, attached to the release
# Usage: scripts/flatpak-repo.sh <gpg-key-id> <base-url> <site> <bundles> <repo>...
# GNUPGHOME selects the keyring holding the private key.
set -euo pipefail

key=$1 base=${2%/} site=$3 bundles=$4
shift 4
app=io.github.felixilgatto.Nuage
branch=master
gpg_home=(--gpg-homedir="${GNUPGHOME:-$HOME/.gnupg}")
# Signing starts a gpg-agent: stop it, or it outlives the script.
trap 'gpgconf --kill gpg-agent 2>/dev/null || true' EXIT

rm -rf "$site"
mkdir -p "$site" "$bundles"
gpg --export "$key" > "$site/nuage.gpg"
pubkey=$(base64 -w0 "$site/nuage.gpg")

# One repository with every architecture, each commit signed.
ostree init --mode=archive-z2 --repo="$site/repo"
for src in "$@"; do
    for ref in $(ostree refs --repo="$src" | grep -E "^(app|runtime)/"); do
        flatpak build-commit-from --src-repo="$src" --gpg-sign="$key" "${gpg_home[@]}" \
            --no-update-summary "$site/repo" "$ref"
    done
done
flatpak build-update-repo --title=Nuage --default-branch="$branch" \
    --homepage="$base/" \
    --gpg-sign="$key" "${gpg_home[@]}" --generate-static-deltas --prune "$site/repo"

cat > "$site/nuage.flatpakref" <<EOF
[Flatpak Ref]
Name=$app
Branch=$branch
Title=Nuage
Url=$base/repo/
SuggestRemoteName=nuage
RuntimeRepo=https://flathub.org/repo/flathub.flatpakrepo
IsRuntime=false
Homepage=$base/
GPGKey=$pubkey
EOF

cat > "$site/nuage.flatpakrepo" <<EOF
[Flatpak Repo]
Title=Nuage
Url=$base/repo/
Homepage=$base/
Comment=Nuage: your cloud storage as a regular folder
GPGKey=$pubkey
EOF

for arch in $(ostree refs --repo="$site/repo" | sed -n "s|^app/$app/\([^/]*\)/$branch\$|\1|p"); do
    flatpak build-bundle --arch="$arch" --repo-url="$base/repo/" \
        --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
        --gpg-keys="$site/nuage.gpg" "$site/repo" "$bundles/nuage-$arch.flatpak" "$app" "$branch"
done

"$(dirname "$0")/site-page.sh" "$base" "$site"
# The whole site, kept with the release: the Site workflow republishes the
# page with this repository, unchanged.
tar -C "$site" -cf "$bundles/nuage-site.tar" .
echo "site in $site, bundles in $bundles:"
ls -l "$bundles"
