#!/usr/bin/env bash
# Smoke test of the native package in a clean container (run as root inside
# it): the .deb on Debian/Ubuntu, the .rpm on Fedora/openSUSE, for the
# container's architecture.
#   docker run --rm --device /dev/fuse --cap-add SYS_ADMIN \
#     --security-opt apparmor:unconfined -v "$PWD/target/dist:/dist:ro" \
#     -v "$PWD/scripts:/scripts:ro" ubuntu:24.04 /scripts/smoke-native.sh
# Installs the package, then as a normal user: starts nuaged, checks
# `nuage status --json`, mounts a `local` account and reads through it.
set -euo pipefail
arch=$(uname -m)
if command -v apt-get >/dev/null; then
    export DEBIAN_FRONTEND=noninteractive
    deb_arch=$(dpkg --print-architecture)
    apt-get update -qq >/dev/null
    apt-get install -y -qq /dist/nuage_*_"$deb_arch".deb >/dev/null
    dpkg -s nuage | grep -E "^(Version|Architecture|Depends):"
elif command -v dnf >/dev/null; then
    dnf install -y -q /dist/nuage-*."$arch".rpm shadow-utils util-linux >/dev/null
    rpm -q --qf "Version: %{VERSION}\nArchitecture: %{ARCH}\n" nuage
    rpm -q --requires nuage | grep -q fuse3
elif command -v zypper >/dev/null; then
    zypper --non-interactive --no-gpg-checks install /dist/nuage-*."$arch".rpm shadow util-linux >/dev/null
    rpm -q nuage
else
    echo "no supported package manager" >&2
    exit 1
fi
test -x /usr/lib/nuage/rclone
nuage --version
useradd -m tester
su tester -s /bin/bash -c '
set -euo pipefail
export XDG_RUNTIME_DIR=/tmp/runtime-tester
mkdir -p -m 700 "$XDG_RUNTIME_DIR"
nuaged 2>/tmp/nuaged.log &
for _ in $(seq 50); do nuage daemon status >/dev/null 2>&1 && break; sleep 0.2; done
nuage daemon status
nuage status --json | grep -q "\"config_encrypted\": false"
mkdir -p ~/files && echo hello > ~/files/hello.txt
nuage account add local --name Smoke --param root="$HOME/files"
test "$(cat ~/Cloud/Smoke/hello.txt)" = hello
echo written > ~/Cloud/Smoke/new.txt
for _ in $(seq 50); do [ -f ~/files/new.txt ] && break; sleep 0.2; done
test "$(cat ~/files/new.txt)" = written
nuage --json status | grep -q "\"mounted\": true"
nuage daemon stop --now
' || { cat /tmp/nuaged.log; exit 1; }
echo "smoke test passed"
