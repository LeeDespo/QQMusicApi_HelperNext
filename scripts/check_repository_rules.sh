#!/usr/bin/env bash
set -euo pipefail

failures=0
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; failures=$((failures + 1)); }

active_docs=(README.md AGENTS.md docs/README.md docs/RELEASING.md docs/architecture.md docs/development.md docs/endpoints.md docs/ffi.md docs/parsing.md docs/pending.md docs/testing.md)

hits="$(find src \( -name '*_tmp.*' -o -name '*.hex' -o -name '*.json' \) -print)"
[ -z "$hits" ] && pass 'src/ has no temporary/data fixture files' || { fail 'src/ contains temporary/data fixture files:'; printf '%s\n' "$hits"; }

hits="$(git ls-files -- dist target)"
[ -z "$hits" ] && pass 'dist/ and target/ are not git-tracked' || { fail 'dist/ or target/ is git-tracked:'; printf '%s\n' "$hits"; }

hits="$(git ls-files | grep -E '(^|/)\.(zcode|codex)(/|$)|(^|/)\.zcodeignore$' || true)"
[ -z "$hits" ] && pass 'local agent workspaces are not git-tracked' || { fail 'local agent workspace files are git-tracked:'; printf '%s\n' "$hits"; }

hits="$(git ls-files | grep -E '(^|/)(helpernext\.lock\.json|source\.patch)$' || true)"
[ -z "$hits" ] && pass 'no consumer lock/patch artifacts are tracked' || { fail 'consumer deployment artifacts are tracked:'; printf '%s\n' "$hits"; }

if [ ! -e release_plan.md ] && [ ! -e docs/release.md ] && [ -f docs/RELEASING.md ]; then
  pass 'release truth is docs/RELEASING.md only'
else
  fail 'release doc layout changed: expected docs/RELEASING.md only'
fi

boundary_pattern='NeuMusic|Music_app|dist/reference/|\.zcode/|app/helpernext/|source\.patch|移植层|尚未移植|qqmusic-helper-next-cli|工作单要求'
hits="$(grep -nE "$boundary_pattern" "${active_docs[@]}" || true)"
[ -z "$hits" ] && pass 'active docs contain no consumer/local-reference dependencies' || { fail 'active docs contain consumer/local-reference dependencies:'; printf '%s\n' "$hits"; }

# Production source comments describe the current component, not migration scaffolding.
source_residue_pattern='dist/reference/|docs/qqmusic|Python helper|old helper|被替换的 Python|NeuMusic|Music_app'
hits="$(grep -rnE "$source_residue_pattern" src --include='*.rs' || true)"
[ -z "$hits" ] && pass 'production source contains no migration/local-reference residue' || { fail 'production source contains migration/local-reference residue:'; printf '%s\n' "$hits"; }

hits="$(grep -nE 'v?[0-9]+\.[0-9]+\.[0-9]+' "${active_docs[@]}" || true)"
[ -z "$hits" ] && pass 'active docs contain no hardcoded semantic versions' || { fail 'active docs hardcode semantic versions:'; printf '%s\n' "$hits"; }

if grep -Fq 'pub const COMPONENT_VERSION: &str = env!("CARGO_PKG_VERSION");' src/methods.rs; then
  pass 'component version derives from Cargo.toml'
else
  fail 'COMPONENT_VERSION must derive from CARGO_PKG_VERSION'
fi

secret_pattern='(qm_keyst|musickey|encrypt_uin|qqmusic_key)[[:space:]]*=[[:space:]]*["'"'"'][^"'"'"']{8,}["'"'"']'
hits="$(git grep -inE "$secret_pattern" -- . || true)"
[ -z "$hits" ] && pass 'no literal secret assignments in tracked files' || { fail 'possible literal secret assignments in tracked files:'; printf '%s\n' "$hits"; }

link_failures=""
for doc in "${active_docs[@]}"; do
  doc_dir="$(dirname "$doc")"
  targets="$(grep -o '](\([^)]*\))' "$doc" 2>/dev/null | sed -e 's/^\](//' -e 's/)$//' || true)"
  while IFS= read -r target; do
    [ -z "$target" ] && continue
    case "$target" in http://*|https://*|mailto:*|'#'*) continue ;; esac
    path="${target%%#*}"
    path="${path%% *}"
    [ -z "$path" ] && continue
    [ -e "$doc_dir/$path" ] || link_failures="$link_failures\n  $doc -> $target"
  done <<< "$targets"
done
[ -z "$link_failures" ] && pass 'relative links in active docs resolve' || { fail 'unresolved relative links in active docs:'; printf '%b\n' "$link_failures"; }

if grep -q 'license = "GPL-3.0-or-later"' Cargo.toml && grep -q 'GPL-3.0-or-later' README.md && head -3 LICENSE | grep -qi 'GNU GENERAL PUBLIC LICENSE'; then
  pass 'license wording consistent'
else
  fail 'license wording inconsistent across Cargo.toml / README / LICENSE'
fi

[ "$failures" -eq 0 ] && { printf 'all repository rule checks passed\n'; exit 0; }
printf '%d check(s) failed\n' "$failures"
exit 1
