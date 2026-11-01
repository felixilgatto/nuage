#!/bin/sh
# Installs Nuage for the current user, without root:
#   ~/.local/bin/{nuaged,nuage}, ~/.local/lib/nuage/rclone,
#   ~/.config/systemd/user/nuaged.service
# Usage: ./install.sh [--uninstall]
set -eu

here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin"
lib="$HOME/.local/lib/nuage"
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
doc="$HOME/.local/share/doc/nuage"

have_systemd() { command -v systemctl >/dev/null && systemctl --user show-environment >/dev/null 2>&1; }

if [ "${1:-}" = "--uninstall" ]; then
    if have_systemd; then
        systemctl --user disable --now nuaged 2>/dev/null || true
    fi
    rm -f "$bin/nuaged" "$bin/nuage" "$units/nuaged.service"
    rm -rf "$lib" "$doc"
    have_systemd && systemctl --user daemon-reload || true
    echo "Nuage is uninstalled. Your settings and cache are kept in ~/.config/nuage and ~/.cache/nuage."
    exit 0
fi

mkdir -p "$bin" "$lib" "$units" "$doc"
install -m 755 "$here/nuaged" "$here/nuage" "$bin/"
install -m 755 "$here/rclone" "$lib/rclone"
sed "s|^ExecStart=.*|ExecStart=$bin/nuaged|" "$here/nuaged.service" > "$units/nuaged.service"
install -m 644 "$here/LICENSE" "$here/rclone-COPYING" "$here/README.md" \
    "$here/SERVER.md" "$here/PROVIDERS.md" "$here/GOOGLE_OAUTH.md" "$here/CLI.md" "$here/TROUBLESHOOTING.md" "$doc/"

echo "Nuage is installed in $bin."
if ! command -v fusermount3 >/dev/null; then
    echo "Warning: fusermount3 is missing. Install it (Debian, Ubuntu: sudo apt install fuse3)."
fi
case ":$PATH:" in
    *":$bin:"*) ;;
    *) echo "Note: add $bin to your PATH to run 'nuage' (most distributions do it at the next login)." ;;
esac
if have_systemd; then
    systemctl --user daemon-reload
    echo "Start it now and at every login with:"
    echo "  systemctl --user enable --now nuaged"
    echo "On a server, keep it running without a session with:"
    echo "  loginctl enable-linger \"\$USER\""
else
    echo "systemd is not available: start the daemon with '$bin/nuaged &'."
fi
