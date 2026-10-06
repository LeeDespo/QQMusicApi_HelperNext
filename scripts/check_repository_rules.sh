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

# 5. 根目录不得出现第二 release 真源（唯一真源是 docs/release.md）。
if [ ! -e release_plan.md ]; then
    pass 'no release_plan.md at repo root'
else
    fail 'release_plan.md exists at repo root (release truth lives in docs/release.md)'
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

if [ "$failures" -eq 0 ]; then
    printf 'all repository rule checks passed\n'
    exit 0
fi
printf '%d check(s) failed\n' "$failures"
exit 1
