#!/usr/bin/env bash
# 移植层的活体冒烟检查。
#
# 用真实凭据把移植进来的**只读**接口逐个打一遍，把结果汇总成一行 JSON 输出到
# 标准输出的 `SMOKE_RESULT ` 前缀行（工作流按这行解析，不解析人类日志）。
#
# 用法： scripts/port-smoke.sh [helper-dir] [method ...]
#   helper-dir  凭据目录（含 Credential/qqmusic-credential.json）；
#               不给就用组件自己的默认目录（macOS 上是
#               ~/Library/Application Support/kmgccc.player/QQMusicHelperNext）
#   method...   只检查这些方法；不给则检查脚本里 METHODS 列出的全部
#
# 约定（照抄参考库 tests/ 的跑法）：
#   * 只调只读接口。写接口（收藏、建歌单、发评论）一律不在这里触发，
#     它们的正确性靠单测与文档，不靠打真实账号。
#   * 登录过期/风控(2001/1000)不算失败，记为 skipped —— 那是环境问题。
#
# 退出码：failed 为 0 时 0，否则 1。

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$REPO_ROOT/target/debug/qqmusic-helper-next"

# 自己保证二进制是新的：只跑 cargo test 的调用方不会构建 bin 目标，
# 用旧二进制冒烟会得到"明明移植了却说不支持"的假失败。
if ! (cd "$REPO_ROOT" && cargo build --bin qqmusic-helper-next >/dev/null 2>&1); then
  echo "SMOKE_RESULT {\"passed\":0,\"failed\":1,\"skipped\":0,\"failures\":[{\"method\":\"cargo build\",\"reason\":\"二进制构建失败\"}],\"checked\":[]}"
  exit 1
fi
if [ ! -x "$BIN" ]; then
  echo "SMOKE_RESULT {\"passed\":0,\"failed\":1,\"skipped\":0,\"failures\":[{\"method\":\"cargo build\",\"reason\":\"构建后仍找不到二进制\"}],\"checked\":[]}"
  exit 1
fi

HELPER_DIR=""
if [ "${1:-}" != "" ] && [ -d "${1:-}" ]; then
  HELPER_DIR="$1"
  shift
fi
if [ -n "$HELPER_DIR" ]; then
  export QQMUSIC_HELPER_NEXT_DIR="$HELPER_DIR"
else
  # 不设任何变量：组件自己解析默认目录，找不到凭据时下面的调用会回
  # "登录"-类错误并按环境问题跳过。
  unset QQMUSIC_HELPER_NEXT_DIR
fi

if [ "$#" -gt 0 ]; then
  METHODS=("$@")
else
  METHODS=(
    fetch_comment_count fetch_hot_comments fetch_new_comments
    fetch_mv_detail resolve_mv_urls fetch_mv_list
    fetch_singer_list fetch_singer_index fetch_similar_artists fetch_artist_tab
    fetch_artist_display_name fetch_artist_mvs
    query_songs fetch_cdn_dispatch fetch_song_producer fetch_song_fav_count
    fetch_similar_songs fetch_song_labels fetch_related_playlists fetch_related_mvs
    fetch_fav_playlists fetch_fav_albums fetch_fav_mvs fetch_music_gene
    fetch_dislike_list
    fetch_user_homepage fetch_vip_info fetch_follow_singers fetch_fans
    fetch_friends fetch_followed_users fetch_created_playlists
    fetch_home_feed fetch_radar_recommend fetch_recommend_playlists
    fetch_search_hotkeys complete_search quick_search general_search
    fetch_wx_qrcode
  )
fi

# 每个方法的参数（JSON）。参数从参考库的 tests/ 与文档里来，取值尽量是
# 长期存在的公开对象。
params_for() {
  case "$1" in
    fetch_comment_count)         echo '{"bizType":1,"bizId":2314161}' ;;
    fetch_hot_comments)          echo '{"bizType":1,"bizId":2314161,"page":1,"limit":5}' ;;
    fetch_new_comments)          echo '{"bizType":1,"bizId":2314161,"page":1,"limit":5}' ;;
    fetch_mv_detail)            echo '{"vids":["000qrPik2w6lDr"]}' ;;
    resolve_mv_urls)            echo '{"vids":["000qrPik2w6lDr"]}' ;;
    fetch_mv_list)              echo '{"area":15,"version":7,"order":0,"limit":5,"page":1}' ;;
    fetch_singer_list)          echo '{"limit":10}' ;;
    fetch_singer_index)         echo '{"page":1,"limit":20}' ;;
    fetch_similar_artists)      echo '{"singerMid":"0025NhlN2yWrP4","limit":5}' ;;
    fetch_artist_tab)           echo '{"singerMid":"0025NhlN2yWrP4","tabType":1,"page":1,"limit":5}' ;;
    fetch_artist_display_name)  echo '{"singerMid":"0025NhlN2yWrP4"}' ;;
    fetch_artist_mvs)           echo '{"singerMid":"0025NhlN2yWrP4","limit":5,"page":1}' ;;
    query_songs)                echo '{"songs":[{"mid":"003w2xz20QlUZt"}]}' ;;
    fetch_cdn_dispatch)         echo '{}' ;;
    fetch_song_producer)        echo '{"songMid":"003w2xz20QlUZt"}' ;;
    fetch_song_fav_count)       echo '{"songIds":[2314161]}' ;;
    fetch_similar_songs)        echo '{"songId":2314161}' ;;
    fetch_song_labels)          echo '{"songId":2314161}' ;;
    fetch_related_playlists)    echo '{"songId":2314161}' ;;
    fetch_related_mvs)          echo '{"songId":2314161}' ;;
    fetch_fav_playlists)        echo '{}' ;;
    fetch_fav_albums)           echo '{}' ;;
    fetch_fav_mvs)              echo '{}' ;;
    fetch_music_gene)           echo '{}' ;;
    fetch_dislike_list)         echo '{}' ;;
    fetch_user_homepage)        echo '{}' ;;
    fetch_vip_info)             echo '{}' ;;
    fetch_follow_singers)       echo '{}' ;;
    fetch_fans)                 echo '{}' ;;
    fetch_friends)              echo '{}' ;;
    fetch_followed_users)       echo '{}' ;;
    fetch_created_playlists)    echo '{}' ;;
    fetch_home_feed)            echo '{"page":1}' ;;
    fetch_radar_recommend)      echo '{"page":1}' ;;
    fetch_recommend_playlists)  echo '{"page":1,"limit":5}' ;;
    fetch_search_hotkeys)       echo '{}' ;;
    complete_search)            echo '{"keyword":"周杰伦"}' ;;
    quick_search)               echo '{"keyword":"周杰伦"}' ;;
    general_search)             echo '{"keyword":"周杰伦","limit":5,"page":1}' ;;
    fetch_wx_qrcode)            echo '{}' ;;
    *)                          echo '{}' ;;
  esac
}

passed=0; failed=0; skipped=0
failures=""; checked=""
append_failure() {
  local method="$1" reason="$2"
  reason="${reason//\\/\\\\}"; reason="${reason//\"/\\\"}"
  reason="${reason//$'\n'/ }"; reason="${reason:0:160}"
  if [ -n "$failures" ]; then failures="$failures,"; fi
  failures="$failures{\"method\":\"$method\",\"reason\":\"$reason\"}"
}

for method in "${METHODS[@]}"; do
  params="$(params_for "$method")"
  request="{\"id\":\"smoke\",\"method\":\"$method\",\"params\":$params}"
  reply="$(printf '%s\n' "$request" | "$BIN" 2>/dev/null | head -1)"
  if [ -z "$reply" ]; then
    failed=$((failed+1)); append_failure "$method" "没有回值（进程异常退出）"; continue
  fi
  if printf '%s' "$reply" | grep -q '"ok":true'; then
    passed=$((passed+1))
  else
    error="$(printf '%s' "$reply" | sed -n 's/.*"error":"\([^"]*\)".*/\1/p')"
    case "$error" in
      # 环境问题：登录过期、风控、上游暂时不可用。跳过，不算失败。
      *登录*|*凭据*|*风控*|*过期*|*权限*|*资源不存在*)
        skipped=$((skipped+1)) ;;
      # 其余都是真问题：没移植（"不支持"）、签名算错、解析失败、参数写错。
      *)
        failed=$((failed+1)); append_failure "$method" "${error:-未知错误}" ;;
    esac
  fi
  if [ -n "$checked" ]; then checked="$checked,"; fi
  checked="$checked\"$method\""
done

echo "SMOKE_RESULT {\"passed\":$passed,\"failed\":$failed,\"skipped\":$skipped,\"failures\":[$failures],\"checked\":[$checked]}"
[ "$failed" -eq 0 ]
