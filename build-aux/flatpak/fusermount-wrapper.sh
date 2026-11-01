#!/bin/sh
# Installed as /app/bin/fusermount3 in the Nuage Flatpak. It relays ONLY to
# the host's fusermount3; Nuage makes no other use of flatpak-spawn --host.
#
# The sandbox has neither fusermount3 (setuid) nor the right to mount. When
# rclone or nuage-fs mount, libfuse runs this script with a socket in
# $_FUSE_COMMFD and expects the /dev/fuse descriptor back on it. The mount
# is done on the host by nuage-fuse-relay, shipped in the Flatpak and run
# from the application's host path (app-path in /.flatpak-info, through the
# deploy's "active" link: an update removes the running version's own
# directory, and mounting would fail until a restart). The relay runs the
# host's fusermount3 with a socket of its own (Ubuntu 26.04's AppArmor
# profile refuses sockets inherited from the sandbox) and passes the
# descriptor back. See crates/fuse-relay.
set -eu

if [ -n "${_FUSE_COMMFD:-}" ]; then
    case "$_FUSE_COMMFD" in
        *[!0-9]*|0|1|2) echo "fusermount3 wrapper: bad _FUSE_COMMFD" >&2; exit 1 ;;
    esac
    app_path=$(sed -n 's/^app-path=//p' /.flatpak-info)
    if [ -z "$app_path" ]; then
        echo "fusermount3 wrapper: app-path not found in /.flatpak-info" >&2
        exit 1
    fi
    # .../<branch>/<commit>/files -> .../<branch>/active/files
    deploy=${app_path%/files}
    relay="${deploy%/*}/active/files/bin/nuage-fuse-relay"
    exec flatpak-spawn --host --watch-bus \
        --env=_FUSE_COMMFD="$_FUSE_COMMFD" --forward-fd="$_FUSE_COMMFD" \
        "$relay" "$@"
fi
# Unmount (-u [-z]), --version: no descriptor to pass.
exec flatpak-spawn --host --watch-bus fusermount3 "$@"
