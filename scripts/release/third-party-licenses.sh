#!/usr/bin/env bash
# Generate THIRD-PARTY-LICENSES.txt (docs/release.md §11).
#
# Identifier-level summary of every dependency the lockfile resolves, taken from
# `cargo metadata --locked`: the same Cargo.lock always produces the same file.
# The full text of each license is not vendored here; the repository column is
# what a consumer follows to read it.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="${1:-$root/dist/release/THIRD-PARTY-LICENSES.txt}"
mkdir -p "$(dirname "$out")"

# Prefer the rustup shims, so this agrees with the packaging scripts about which
# cargo resolves the lockfile.
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

python3 - "$root" "$out" <<'PY'
import json
import subprocess
import sys

root, out = sys.argv[1], sys.argv[2]
meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=root, text=True))
own = next(p for p in meta["packages"] if p["name"] == "qqmusic-api-helper-next")
deps = sorted((p for p in meta["packages"] if p["source"]),
              key=lambda p: (p["name"].lower(), p["version"]))

lines = [
    "QQMusicApi_HelperNext third-party licenses",
    "=" * 72,
    "",
    f"Component version: {own['version']}",
    "Generated from Cargo.lock by scripts/release/third-party-licenses.sh.",
    "Each entry names the license identifier the dependency declares; the full",
    "text is available from the repository or the crate's crates.io page.",
    "",
    "This component is GPL-3.0-or-later (see LICENSE). It is a Rust port of",
    "QQMusicApi (https://github.com/L-1124/QQMusicApi), also GPL-3.0-or-later.",
    "",
    "=" * 72,
    "",
]
for dep in deps:
    lines.append(f"{dep['name']} {dep['version']}")
    lines.append(f"  license:    {dep.get('license') or 'not declared in Cargo.toml'}")
    lines.append(f"  repository: {dep.get('repository') or '-'}")
    lines.append("")

with open(out, "w") as handle:
    handle.write("\n".join(lines).rstrip() + "\n")
print(f"wrote {out} ({len(deps)} dependencies)")
PY
