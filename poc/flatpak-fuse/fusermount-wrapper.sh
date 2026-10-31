#!/bin/sh
# Installed as /app/bin/fusermount3. Relays ONLY to the host's fusermount3.
#
# On mount, libfuse hands us a socket in $_FUSE_COMMFD and expects the
# /dev/fuse fd back over it. Ubuntu 26.04's AppArmor profile for fusermount3
# refuses sockets inherited from a sandboxed process (peer=unpriv_bwrap), so a
# relay running on the host creates its own socketpair for fusermount3 and
# passes the received fd back through the sandbox's socket.
set -eu

RELAY='
import array, os, socket, subprocess, sys
comm = socket.socket(fileno=int(os.environ["_FUSE_COMMFD"]))
ours, theirs = socket.socketpair()
env = dict(os.environ, _FUSE_COMMFD=str(theirs.fileno()))
proc = subprocess.Popen(["fusermount3", *sys.argv[1:]], pass_fds=(theirs.fileno(),), env=env)
theirs.close()
fds = array.array("i")
msg, anc, _, _ = ours.recvmsg(1, socket.CMSG_SPACE(fds.itemsize))
for level, kind, data in anc:
    if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
        fds.frombytes(data[: len(data) - len(data) % fds.itemsize])
if fds:
    comm.sendmsg([msg or b"\0"], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, fds)])
sys.exit(proc.wait())
'

if [ -n "${_FUSE_COMMFD:-}" ]; then
    case "$_FUSE_COMMFD" in
        *[!0-9]*|0|1|2) echo "fusermount3 wrapper: bad _FUSE_COMMFD" >&2; exit 1 ;;
    esac
    exec flatpak-spawn --host --watch-bus \
        --env=_FUSE_COMMFD="$_FUSE_COMMFD" --forward-fd="$_FUSE_COMMFD" \
        python3 -c "$RELAY" "$@"
fi
# Unmount (-u [-z]), --version: no fd to forward.
exec flatpak-spawn --host --watch-bus fusermount3 "$@"
