#!/usr/bin/env bash
# Assemble the top-level release metadata from the two packages: the release
# manifest (docs/release.md §7) and SHA256SUMS (§8). Runs in the release job,
# after both build jobs have uploaded their artifacts into $RELEASE_DIR.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

version="$(python3 -c 'import sys,tomllib;print(tomllib.load(open(sys.argv[1],"rb"))["package"]["version"])' Cargo.toml)"
protocol="$(python3 -c 'import re,sys;print(re.search(r"PROTOCOL_VERSION:\s*i32\s*=\s*(\d+)",open(sys.argv[1]).read()).group(1))' src/methods.rs)"
release_dir="${RELEASE_DIR:-$root/dist/release}"
tag="v$version"
macos_archive="qqmusic-helper-next-v$version-macos-arm64.tar.gz"
android_archive="qqmusic-helper-next-v$version-android.zip"
licenses="THIRD-PARTY-LICENSES.txt"

for asset in "$macos_archive" "$android_archive" "$licenses"; do
    [ -f "$release_dir/$asset" ] || { echo "missing release asset: $release_dir/$asset" >&2; exit 1; }
done

dirty="$(git status --porcelain)"
if [ -n "$dirty" ] && [ -z "${ALLOW_DIRTY_TREE:-}" ]; then
    echo "refusing to assemble a formal release from a dirty tree (ALLOW_DIRTY_TREE=1 only rehearses)" >&2
    echo "$dirty" >&2
    exit 1
fi

python3 - "$release_dir" "$version" "$tag" "$protocol" <<'PY'
import hashlib, json, pathlib, subprocess, sys

release_dir, version, tag, protocol = sys.argv[1:5]
directory = pathlib.Path(release_dir)


def sha256(name):
    return hashlib.sha256((directory / name).read_bytes()).hexdigest()


def sidecar(name):
    file = directory / name
    return json.loads(file.read_text()) if file.is_file() else {}


macos_archive = f"qqmusic-helper-next-v{version}-macos-arm64.tar.gz"
android_archive = f"qqmusic-helper-next-v{version}-android.zip"
toolchain = {**sidecar("toolchain-macos.json"), **sidecar("toolchain-android.json")}
manifest = {
    "name": "QQMusicApi_HelperNext",
    "version": version,
    "tag": tag,
    "protocolVersion": int(protocol),
    "gitCommit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
    "sourceTreeClean": not subprocess.check_output(["git", "status", "--porcelain"], text=True).strip(),
    "toolchain": toolchain,
    "artifacts": {
        macos_archive: {
            "kind": "stdio-binary",
            "target": "aarch64-apple-darwin",
            "sha256": sha256(macos_archive),
        },
        android_archive: {
            "kind": "boltffi-android",
            "abis": ["arm64-v8a", "armeabi-v7a", "x86", "x86_64"],
            "sha256": sha256(android_archive),
        },
    },
}
(directory / "release-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")

covered = [macos_archive, android_archive, "release-manifest.json", "THIRD-PARTY-LICENSES.txt"]
(directory / "SHA256SUMS").write_text(
    "".join(f"{sha256(name)}  {name}\n" for name in covered))
print(f"assembled {tag}: " + ", ".join(covered))
PY
