#!/usr/bin/env bash
# 仓库结构守门检查：结构规则 + 宽松 secret 扫描。
# 设计目标：简单、可长期运行——只依赖 git/find/grep，模式保持宽松，
# 不做脆弱的字符串黑名单（避免把参数名、文档叙述误判为泄露）。
set -euo pipefail

failures=0

pass() { printf 'ok   %s\n' "$1"; }
fail() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}

# 1. src/ 下不夹带临时文件（临时向量一律迁往 tests/fixtures/）。
hits="$(find src -name '*_tmp.*' -print)"
if [ -z "$hits" ]; then
    pass 'src/ has no *_tmp.* files'
else
    fail 'src/ contains *_tmp.* files:'
    printf '%s\n' "$hits"
fi

# 2. src/ 下不夹带 .hex / .json 数据文件（数据放 tests/fixtures/ 或 dist/，源码只留代码）。
hits="$(find src \( -name '*.hex' -o -name '*.json' \) -print)"
if [ -z "$hits" ]; then
    pass 'src/ has no .hex/.json data files'
else
    fail 'src/ contains .hex/.json data files:'
    printf '%s\n' "$hits"
fi

# 3. dist/ 与 target/ 是构建产物，不得被 git 跟踪。
hits="$(git ls-files -- dist target)"
if [ -z "$hits" ]; then
    pass 'dist/ and target/ are not git-tracked'
else
    fail 'dist/ or target/ is git-tracked:'
    printf '%s\n' "$hits"
fi

# 4. .zcodeignore 是工作区工具配置，不属于仓库内容。
if [ -z "$(git ls-files -- .zcodeignore)" ]; then
    pass '.zcodeignore is not git-tracked'
else
    fail '.zcodeignore is git-tracked'
fi

# 5. release 真源唯一：docs/RELEASING.md 是唯一真源；根目录不得出现 release_plan.md，
#    docs/release.md 不存在（改名后不得回流出第二份）。
if [ ! -e release_plan.md ] && [ ! -e docs/release.md ] && [ -f docs/RELEASING.md ]; then
    pass 'release truth is docs/RELEASING.md only'
else
    fail 'release doc layout changed: expected docs/RELEASING.md, no root release_plan.md, no docs/release.md'
fi

# 6. 宽松 secret 扫描：只识别“秘密名 = 引号包裹的字面值”这一明显泄露形态；
#    参数名引用（如 ("qm_keyst", key)）与文档叙述不在此列。
secret_pattern='(qm_keyst|musickey|encrypt_uin|qqmusic_key)[[:space:]]*=[[:space:]]*["'"'"'][^"'"'"']{8,}["'"'"']'
hits="$(git grep -inE "$secret_pattern" -- . || true)"
if [ -z "$hits" ]; then
    pass 'no literal secret assignments in tracked files'
else
    fail 'possible literal secret assignments in tracked files:'
    printf '%s\n' "$hits"
fi

# 7. 活文档（README / AGENTS / docs 顶层）的相对链接必须可解析。
#    history/ 的历史取证记录不在检查范围；锚点链接与外链跳过。
link_failures=""
for doc in README.md AGENTS.md docs/*.md; do
    doc_dir="$(dirname "$doc")"
    targets="$(grep -o '](\([^)]*\))' "$doc" 2>/dev/null | sed -e 's/^\](//' -e 's/)$//' || true)"
    while IFS= read -r target; do
        [ -z "$target" ] && continue
        case "$target" in
            http://*|https://*|mailto:*|'#'*) continue ;;
        esac
        path="${target%%#*}"
        path="${path%% *}"
        [ -z "$path" ] && continue
        if [ ! -e "$doc_dir/$path" ]; then
            link_failures="$link_failures\n  $doc -> $target"
        fi
    done <<< "$targets"
done
if [ -z "$link_failures" ]; then
    pass 'relative links in live docs resolve'
else
    fail 'unresolved relative links in live docs:'
    printf '%b\n' "$link_failures"
fi

# 8. 活文档不得引用不可导航的本机宿主路径（跨仓引用用 GitHub 链接或 history 摘要）。
hits="$(grep -rn 'Music_app' README.md AGENTS.md docs/*.md || true)"
if [ -z "$hits" ]; then
    pass 'live docs reference no local host paths'
else
    fail 'live docs reference local host paths:'
    printf '%s\n' "$hits"
fi

# 9. 许可证表述一致：Cargo.toml 与 README 均为 GPL-3.0-or-later，LICENSE 为 GPL 文本。
if grep -q 'license = "GPL-3.0-or-later"' Cargo.toml \
   && grep -q 'GPL-3.0-or-later' README.md \
   && head -3 LICENSE | grep -qi 'GNU GENERAL PUBLIC LICENSE'; then
    pass 'license wording consistent (Cargo.toml / README / LICENSE)'
else
    fail 'license wording inconsistent across Cargo.toml / README / LICENSE'
fi

if [ "$failures" -eq 0 ]; then
    printf 'all repository rule checks passed\n'
    exit 0
fi
printf '%d check(s) failed\n' "$failures"
exit 1
