#!/usr/bin/env bash
# Checks that the version is the same everywhere a release needs it:
#   - the workspace version (Cargo.toml),
#   - the newest <release> of the AppStream metainfo,
#   - the newest "## [<version>] - <date>" section of CHANGELOG.md,
#   - with an argument (the tag without its "v"), that version too.
# The metainfo and changelog dates must match. Prints the version.
# Usage: scripts/check-release.sh [<version>]
set -euo pipefail

cd "$(dirname "$0")/.."
metainfo=data/io.github.felixilgatto.Nuage.metainfo.xml

cargo=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
read -r meta meta_date < <(grep -m1 -o '<release version="[^"]*" date="[^"]*"' "$metainfo" |
    sed 's/<release version="\([^"]*\)" date="\([^"]*\)"/\1 \2/')
read -r log log_date < <(grep -m1 -E '^## \[[0-9]' CHANGELOG.md |
    sed -E 's/^## \[([^]]*)\] - ([0-9-]+).*/\1 \2/')

fail=0
check() { # what, value, expected
    if [[ "$2" != "$3" ]]; then
        echo "::error::$1 is \"$2\", expected \"$3\"" >&2
        fail=1
    fi
}
expected=${1:-$cargo}
check "Cargo.toml version" "$cargo" "$expected"
check "metainfo newest release" "$meta" "$expected"
check "CHANGELOG.md newest section" "$log" "$expected"
check "CHANGELOG.md date" "$log_date" "$meta_date"
[[ $fail == 0 ]] || exit 1
echo "$expected"
