#!/usr/bin/env bash
# Makes the Flatpak repository published on GitHub Pages, from the OSTree
# repositories flatpak-builder made (one per architecture):
#   <site>/repo/             the signed repository (`flatpak update` reads it)
#   <site>/nuage.flatpakref  one-click install, also adds the repository
#   <site>/nuage.flatpakrepo the repository alone
#   <site>/index.html        how to install
#   <bundles>/nuage-<arch>.flatpak  bundles that update from the repository
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
    --homepage=https://github.com/felixilgatto/nuage \
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
Homepage=https://github.com/felixilgatto/nuage
GPGKey=$pubkey
EOF

cat > "$site/nuage.flatpakrepo" <<EOF
[Flatpak Repo]
Title=Nuage
Url=$base/repo/
Homepage=https://github.com/felixilgatto/nuage
Comment=Nuage: your cloud storage as a regular folder
GPGKey=$pubkey
EOF

for arch in $(ostree refs --repo="$site/repo" | sed -n "s|^app/$app/\([^/]*\)/$branch\$|\1|p"); do
    flatpak build-bundle --arch="$arch" --repo-url="$base/repo/" \
        --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
        --gpg-keys="$site/nuage.gpg" "$site/repo" "$bundles/nuage-$arch.flatpak" "$app" "$branch"
done

cat > "$site/index.html" <<EOF
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Install Nuage</title>
<style>
  :root { color-scheme: light dark; --bg: #fafafa; --fg: #1c1c1c; --muted: #5c5c5c; --code: #ececec; --accent: #1c5fb8; }
  @media (prefers-color-scheme: dark) { :root { --bg: #1d1d20; --fg: #ececec; --muted: #a8a8a8; --code: #2c2c30; --accent: #8fb8f0; } }
  body { margin: 0; background: var(--bg); color: var(--fg); font: 16px/1.55 system-ui, sans-serif; }
  main { max-width: 44rem; margin: 0 auto; padding: 2.5rem 1rem; }
  h1 { margin-top: 0; } a { color: var(--accent); } p.lead { color: var(--muted); }
  pre { background: var(--code); padding: .75rem 1rem; border-radius: 8px; overflow-x: auto; }
  code { font-family: ui-monospace, monospace; font-size: .92em; }
</style>
</head>
<body>
<main>
<h1>Install Nuage</h1>
<p class="lead">Your cloud storage as a regular folder on Linux.</p>
<p>You need Flatpak (on Ubuntu: <code>sudo apt install flatpak</code>). Then open
<a href="nuage.flatpakref">nuage.flatpakref</a> with Software, or run:</p>
<pre><code>flatpak install --user $base/nuage.flatpakref</code></pre>
<p>Updates come with <code>flatpak update</code> and in Software, like any other app.
The GNOME runtime comes from Flathub, added on the way if needed.</p>
<p>Servers, other packages, source: <a href="https://github.com/felixilgatto/nuage">github.com/felixilgatto/nuage</a>.</p>
</main>
</body>
</html>
EOF
echo "site in $site, bundles in $bundles:"
ls -l "$bundles"
