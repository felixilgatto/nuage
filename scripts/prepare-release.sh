#!/usr/bin/env bash
# Prepares a release, without tagging or pushing anything:
#   - sets the workspace version in Cargo.toml (and Cargo.lock);
#   - turns "## [Unreleased]" of CHANGELOG.md into "## [<version>] - <date>"
#     (a new, empty "## [Unreleased]" stays on top); run again for the same
#     version to only update the date;
#   - adds or updates the <release> of the AppStream metainfo, with the
#     changelog items of that version.
# Then review the diff, commit, and push the tag v<version> to release.
# Usage: scripts/prepare-release.sh <version> [<date, default today>]
set -euo pipefail

version=${1:?usage: scripts/prepare-release.sh <version> [<date>]}
date=${2:-$(date +%F)}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "not a version: $version" >&2; exit 2; }
[[ $date =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || { echo "not a date: $date" >&2; exit 2; }

cd "$(dirname "$0")/.."
metainfo=data/io.github.felixilgatto.Nuage.metainfo.xml

sed -i "/^\[workspace.package\]/,/^\[/s/^version = \".*\"/version = \"$version\"/" Cargo.toml
cargo update --workspace --quiet

python3 -I - "$version" "$date" "$metainfo" <<'EOF'
import html, re, sys

version, date, metainfo = sys.argv[1:]

# CHANGELOG.md
text = open("CHANGELOG.md").read()
section = re.compile(r"^## \[" + re.escape(version) + r"\][^\n]*$", re.M)
if section.search(text):
    text = section.sub(f"## [{version}] - {date}", text, count=1)
else:
    if "## [Unreleased]" not in text:
        sys.exit("CHANGELOG.md has no [Unreleased] section")
    text = text.replace("## [Unreleased]", f"## [Unreleased]\n\n## [{version}] - {date}", 1)
open("CHANGELOG.md", "w").write(text)

# Items of this version: its top-level bullets, without Markdown.
body = re.search(r"^## \[" + re.escape(version) + r"\][^\n]*\n(.*?)(?=^## \[|\Z)", text, re.M | re.S).group(1)
items = []
for bullet in re.split(r"^- ", body, flags=re.M)[1:]:
    line = " ".join(bullet.split())
    line = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", line).replace("`", "")
    line = re.sub(r"(?<!\w)(\*\*|__|\*|_)(\S(?:.*?\S)?)\1(?!\w)", r"\2", line)
    items.append(html.escape(line, quote=False))
if not items:
    sys.exit(f"CHANGELOG.md: the {version} section is empty")
lis = "\n".join(f"          <li>{i}</li>" for i in items)
release = (f'    <release version="{version}" date="{date}">\n'
           f"      <description>\n        <ul>\n{lis}\n        </ul>\n      </description>\n"
           f"    </release>\n")

# metainfo: replace this version's <release>, or add it first.
xml = open(metainfo).read()
existing = re.compile(r'    <release version="' + re.escape(version) + r'".*?</release>\n', re.S)
if existing.search(xml):
    xml = existing.sub(lambda _: release, xml, count=1)
else:
    xml = xml.replace("  <releases>\n", "  <releases>\n" + release, 1)
open(metainfo, "w").write(xml)
EOF

scripts/check-release.sh "$version" >/dev/null
echo "Prepared $version ($date). Review with 'git diff', commit, then: git tag v$version && git push origin v$version"
