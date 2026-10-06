#!/usr/bin/env bash
# Build and lay out the Android package (docs/release.md §5, §6).
#
# Output in $RELEASE_DIR (default dist/release):
#   qqmusic-helper-next-v<version>-android.zip
#   toolchain-android.json — what the release job folds into release-manifest.json
#
# Kotlin binding, JNI glue and the four ABI libraries come from this one
# `boltffi pack` run: the set is never assembled from separate builds.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# Prefer the rustup shims: a Homebrew/distro cargo earlier on PATH does not see
# the rustup Android standard libraries and compiles with a different toolchain
# than the one the manifest records.
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

if [ -z "${ALLOW_DIRTY_TREE:-}" ] && [ -n "$(git status --porcelain)" ]; then
    echo "refusing to pack a formal artifact from a dirty tree (ALLOW_DIRTY_TREE=1 only rehearses)" >&2
    exit 1
fi

version="$(python3 -c 'import sys,tomllib;print(tomllib.load(open(sys.argv[1],"rb"))["package"]["version"])' Cargo.toml)"
protocol="$(python3 -c 'import re,sys;print(re.search(r"PROTOCOL_VERSION:\s*i32\s*=\s*(\d+)",open(sys.argv[1]).read()).group(1))' src/methods.rs)"
release_dir="${RELEASE_DIR:-$root/dist/release}"
package_name="qqmusic-helper-next-v$version-android"
stage="$release_dir/$package_name"
abis=(arm64-v8a armeabi-v7a x86 x86_64)

: "${ANDROID_HOME:=$HOME/Library/Android/sdk}"
export ANDROID_HOME
: "${ANDROID_NDK_HOME:=$ANDROID_HOME/ndk/27.3.13750724}"
export ANDROID_NDK_HOME
[ -d "$ANDROID_NDK_HOME" ] || { echo "Android NDK not found: $ANDROID_NDK_HOME" >&2; exit 1; }
ndk_version="$(basename "$ANDROID_NDK_HOME")"
boltffi_version="$(boltffi --version | awk '{print $NF}')"

scripts/release/third-party-licenses.sh "$release_dir/THIRD-PARTY-LICENSES.txt"

# --deny-skipped: a binding with declarations left out must fail the release
# rather than ship a package that silently lacks an API.
boltffi pack android --release --deny-skipped

libraries=()
for abi in "${abis[@]}"; do
    library="dist/android/jniLibs/$abi/libqqmusic_api_helper_next.so"
    [ -f "$library" ] || { echo "missing ABI: $abi" >&2; exit 1; }
    libraries+=("$library")
done
python3 scripts/release/check_16kb_pages.py "${libraries[@]}"

rm -rf "$stage"
mkdir -p "$stage"
cp -R dist/android/kotlin "$stage/kotlin"
cp -R dist/android/jniLibs "$stage/jniLibs"
cp LICENSE "$stage/LICENSE"
cp "$release_dir/THIRD-PARTY-LICENSES.txt" "$stage/THIRD-PARTY-LICENSES.txt"

python3 - "$stage" "$version" "$protocol" "$ndk_version" "$boltffi_version" <<'PY'
import hashlib, json, pathlib, subprocess, sys

stage, version, protocol, ndk_version, boltffi_version = sys.argv[1:6]
stage_path = pathlib.Path(stage)


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


files = {}
for directory in ("kotlin", "jniLibs"):
    for path in sorted((stage_path / directory).rglob("*")):
        if path.is_file():
            files[str(path.relative_to(stage_path))] = sha256(path)

manifest = {
    "name": "QQMusicApi_HelperNext",
    "componentVersion": version,
    "protocolVersion": int(protocol),
    "gitCommit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
    "boltffi": boltffi_version,
    "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
    "androidNdk": ndk_version,
    "minSdk": 24,
    "abis": ["arm64-v8a", "armeabi-v7a", "x86", "x86_64"],
    "files": files,
}
(stage_path / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(f"manifest.json: {len(files)} files hashed")
PY

python3 - "$release_dir" "$package_name" "$ndk_version" "$boltffi_version" <<'PY'
import json, pathlib, shutil, subprocess, sys

release_dir, package_name, ndk_version, boltffi_version = sys.argv[1:5]
archive = shutil.make_archive(f"{release_dir}/{package_name}", "zip",
                              root_dir=release_dir, base_dir=package_name)
sidecar = {
    "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
    "cargo": subprocess.check_output(["cargo", "--version"], text=True).strip(),
    "boltffi": boltffi_version,
    "androidNdk": ndk_version,
}
pathlib.Path(release_dir, "toolchain-android.json").write_text(json.dumps(sidecar, indent=2) + "\n")
print(f"packed {archive}")
PY
