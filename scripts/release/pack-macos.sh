#!/usr/bin/env bash
# Build and lay out the macOS ARM64 stdio package (docs/RELEASING.md §4).
#
# Output in $RELEASE_DIR (default dist/release):
#   qqmusic-helper-next-v<version>-macos-arm64.tar.gz
#   toolchain-macos.json   — what the release job folds into release-manifest.json
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# Prefer the rustup shims: a Homebrew/distro cargo earlier on PATH would compile
# with a different toolchain than the one the manifest records.
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

# A formal build is made from a clean commit (docs/RELEASING.md §9.2). The override
# exists so a local rehearsal of the packaging itself is possible.
if [ -z "${ALLOW_DIRTY_TREE:-}" ] && [ -n "$(git status --porcelain)" ]; then
    echo "refusing to pack a formal artifact from a dirty tree (ALLOW_DIRTY_TREE=1 only rehearses)" >&2
    exit 1
fi

version="$(python3 -c 'import sys,tomllib;print(tomllib.load(open(sys.argv[1],"rb"))["package"]["version"])' Cargo.toml)"
protocol="$(python3 -c 'import re,sys;print(re.search(r"PROTOCOL_VERSION:\s*i32\s*=\s*(\d+)",open(sys.argv[1]).read()).group(1))' src/methods.rs)"
target="aarch64-apple-darwin"
release_dir="${RELEASE_DIR:-$root/dist/release}"
package_name="qqmusic-helper-next-v$version-macos-arm64"
stage="$release_dir/$package_name"

scripts/release/third-party-licenses.sh "$release_dir/THIRD-PARTY-LICENSES.txt"

cargo build --release --bin qqmusic-helper-next --target "$target"

rm -rf "$stage"
mkdir -p "$stage"
cp "target/$target/release/qqmusic-helper-next" "$stage/qqmusic-helper-next"
cp LICENSE "$stage/LICENSE"
cp NOTICE "$stage/NOTICE"
cp "$release_dir/THIRD-PARTY-LICENSES.txt" "$stage/THIRD-PARTY-LICENSES.txt"

binary="$stage/qqmusic-helper-next"
description="$(file -b "$binary")"
case "$description" in
    *"Mach-O 64-bit"*arm64*) ;;
    *) echo "not an arm64 Mach-O: $description" >&2; exit 1 ;;
esac

# Version smoke: the component answers before any host configuration exists.
answer="$("$binary" --version)"
[ "$answer" = "qqmusic-helper-next $version (protocol $protocol)" ] \
    || { echo "version smoke failed: $answer" >&2; exit 1; }
info="$(printf '{"id":"1","method":"get_helper_info","params":{}}\n' | "$binary")"
case "$info" in
    *'"helperVersion":"'"$version"'"'*'"protocolVersion":'"$protocol"*) ;;
    *) echo "info smoke failed: $info" >&2; exit 1 ;;
esac

python3 - "$stage" "$version" "$protocol" "$target" "$binary" <<'PY'
import hashlib, json, pathlib, subprocess, sys

stage, version, protocol, target, binary = sys.argv[1:6]
digest = hashlib.sha256(pathlib.Path(binary).read_bytes()).hexdigest()
manifest = {
    "name": "QQMusicApi_HelperNext",
    "componentVersion": version,
    "protocolVersion": int(protocol),
    "gitCommit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
    "target": target,
    "binary": "qqmusic-helper-next",
    "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
    "sha256": digest,
}
pathlib.Path(stage, "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(f"manifest.json: binary sha256 {digest}")
PY

python3 - "$release_dir" "$package_name" <<'PY'
import json, pathlib, subprocess, sys

release_dir, package_name = sys.argv[1:3]
subprocess.check_call(["tar", "-czf", f"{release_dir}/{package_name}.tar.gz",
                       "-C", release_dir, package_name])
sidecar = {
    "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
    "cargo": subprocess.check_output(["cargo", "--version"], text=True).strip(),
}
pathlib.Path(release_dir, "toolchain-macos.json").write_text(json.dumps(sidecar, indent=2) + "\n")
print(f"packed {release_dir}/{package_name}.tar.gz")
PY
