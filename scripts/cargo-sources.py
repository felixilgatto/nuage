#!/usr/bin/env python3
"""Writes build-aux/flatpak/cargo-sources.json from Cargo.lock.

The Flatpak build is offline: flatpak-builder downloads every crate listed
here (with its checksum from Cargo.lock) into cargo/vendor/, and the inline
cargo/config.toml makes cargo use that folder instead of crates.io.

Same output layout as flatpak-cargo-generator, for crates.io dependencies
only (the workspace has no git dependencies; this script refuses them).
Standard library only. CI regenerates the file and fails if it differs.

Usage: scripts/cargo-sources.py [Cargo.lock] [output.json]
"""

import json
import sys
import tomllib

REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
VENDOR = "cargo/vendor"


def main() -> int:
    lock_path = sys.argv[1] if len(sys.argv) > 1 else "Cargo.lock"
    out_path = sys.argv[2] if len(sys.argv) > 2 else "build-aux/flatpak/cargo-sources.json"
    with open(lock_path, "rb") as f:
        lock = tomllib.load(f)

    sources = []
    for package in sorted(lock["package"], key=lambda p: (p["name"], p["version"])):
        source = package.get("source")
        if source is None:
            continue  # a workspace crate
        if source != REGISTRY:
            print(f"unsupported source for {package['name']}: {source}", file=sys.stderr)
            return 1
        name, version, checksum = package["name"], package["version"], package["checksum"]
        dest = f"{VENDOR}/{name}-{version}"
        sources.append(
            {
                "type": "archive",
                "archive-type": "tar-gzip",
                "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                "sha256": checksum,
                "dest": dest,
            }
        )
        sources.append(
            {
                "type": "inline",
                "contents": json.dumps({"package": checksum, "files": {}}),
                "dest": dest,
                "dest-filename": ".cargo-checksum.json",
            }
        )
    sources.append(
        {
            "type": "inline",
            "contents": (
                "[source.vendored-sources]\n"
                f'directory = "{VENDOR}"\n'
                "\n"
                "[source.crates-io]\n"
                'replace-with = "vendored-sources"\n'
            ),
            "dest": "cargo",
            "dest-filename": "config.toml",
        }
    )
    with open(out_path, "w") as f:
        json.dump(sources, f, indent=4)
        f.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
