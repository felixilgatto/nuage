#!/usr/bin/env bash
# Smoke test of the .deb in a clean container (run as root inside it):
#   docker run --rm --device /dev/fuse --cap-add SYS_ADMIN \
#     --security-opt apparmor:unconfined -v "$PWD/target/dist:/dist:ro" \
#     -v "$PWD/scripts:/scripts:ro" ubuntu:24.04 /scripts/smoke-native.sh
# Installs the package, then as a normal user: starts nuaged, checks
# `nuage status --json`, mounts a `local` account and reads through it.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq /dist/nuage_*_amd64.deb >/dev/null
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
