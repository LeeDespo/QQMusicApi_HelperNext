#!/usr/bin/env python3
"""Bounded, read-only live smoke. Never dispatch unlisted methods or login calls."""
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
# This explicit map is the entire dispatch allowlist; do not infer read safety from names.
PARAMS = {
    "fetch_comment_count": {
        "bizType": 1,
        "bizId": 2314161
    },
    "fetch_hot_comments": {
        "bizType": 1,
        "bizId": 2314161,
        "page": 1,
        "limit": 5
    },
    "fetch_new_comments": {
        "bizType": 1,
        "bizId": 2314161,
        "page": 1,
        "limit": 5
    },
    "fetch_mv_detail": {
        "vids": [
            "013xscuH0xlbie"
        ]
    },
    "resolve_mv_urls": {
        "vids": [
            "013xscuH0xlbie"
        ]
    },
    "fetch_mv_list": {
        "area": 15,
        "version": 7,
        "order": 0,
        "limit": 5,
        "page": 1
    },
    "fetch_singer_list": {
        "limit": 10
    },
    "fetch_singer_index": {
        "page": 1,
        "limit": 20
    },
    "fetch_similar_artists": {
        "singerMid": "0025NhlN2yWrP4",
        "limit": 5
    },
    "fetch_artist_tab": {
        "singerMid": "0025NhlN2yWrP4",
        "tabType": 1,
        "page": 1,
        "limit": 5
    },
    "fetch_artist_display_name": {
        "singerMid": "0025NhlN2yWrP4"
    },
    "fetch_artist_mvs": {
        "singerMid": "0025NhlN2yWrP4",
        "limit": 5,
        "page": 1
    },
    "query_songs": {
        "songs": [
            {
                "mid": "003w2xz20QlUZt"
            }
        ]
    },
    "fetch_cdn_dispatch": {},
    "fetch_song_producer": {
        "songMid": "003w2xz20QlUZt"
    },
    "fetch_song_fav_count": {
        "songIds": [
            2314161
        ]
    },
    "fetch_similar_songs": {
        "songId": 2314161
    },
    "fetch_song_labels": {
        "songId": 2314161
    },
    "fetch_related_playlists": {
        "songId": 2314161
    },
    "fetch_related_mvs": {
        "songId": 2314161
    },
    "fetch_fav_playlists": {},
    "fetch_fav_albums": {},
    "fetch_fav_mvs": {},
    "fetch_music_gene": {},
    "fetch_dislike_list": {},
    "fetch_user_homepage": {},
    "fetch_vip_info": {},
    "fetch_follow_singers": {},
    "fetch_fans": {},
    "fetch_friends": {},
    "fetch_followed_users": {},
    "fetch_created_playlists": {},
    "fetch_home_feed": {
        "page": 1
    },
    "fetch_radar_recommend": {
        "page": 1
    },
    "fetch_recommend_playlists": {
        "page": 1,
        "limit": 5
    },
    "fetch_search_hotkeys": {},
    "complete_search": {
        "keyword": "周杰伦"
    },
    "quick_search": {
        "keyword": "周杰伦"
    },
    "general_search": {
        "keyword": "周杰伦",
        "limit": 5,
        "page": 1
    },
    "fetch_recommend_comments": {
        "bizType": 1,
        "bizId": 2314161,
        "page": 1,
        "limit": 5
    },
    "fetch_moment_comments": {
        "bizType": 1,
        "bizId": 2314161,
        "page": 1,
        "limit": 5
    },
    "resolve_song_urls": {
        "items": [
            {
                "mid": "003w2xz20QlUZt",
                "fileType": "MP3_128"
            }
        ]
    },
    "fetch_other_versions": {
        "value": "003w2xz20QlUZt"
    },
    "fetch_sheet_music": {
        "mid": "003w2xz20QlUZt"
    },
    "has_sheet_music": {
        "mid": "003w2xz20QlUZt"
    },
    "search_extra": {
        "keyword": "周杰伦",
        "searchType": 0,
        "limit": 5,
        "page": 1
    }
}
PARAMS.update({
    "fetch_new_albums": {"area": 1, "page": 1, "limit": 5},
    "fetch_singing_annotations": {"songId": 4835784},
    "fetch_multi_style_lyrics": {"songId": 496097762},
    "has_ai_dictionary": {"songId": 7137686},
    "fetch_ai_dictionary": {"songId": 7137686},
    "fetch_user_liked_songs": {"page": 1, "limit": 5},
})
SEARCH_TYPES = (0, 1, 2, 3, 4, 7, 8, 10, 15, 18)
PAGE_TWO = {"fetch_hot_comments", "fetch_new_comments", "fetch_mv_list",
            "fetch_singer_index", "fetch_artist_tab", "fetch_artist_mvs",
            "fetch_recommend_playlists", "general_search", "fetch_new_albums", "fetch_user_liked_songs"}


def result():
    return {"passed": 0, "failed": 0, "skipped": 0, "failures": [],
            "checked": [], "skips": [], "cases": []}


def record(summary, method, case, status, reason, duration=0):
    summary[status] += 1
    diagnostic = {"method": method, "case": case, "reason": reason}
    summary["cases"].append({**diagnostic, "status": status, "durationMs": duration})
    if status == "failed":
        summary["failures"].append(diagnostic)
    elif status == "skipped":
        summary["skips"].append(diagnostic)


def finish(summary):
    print("SMOKE_RESULT " + json.dumps(summary, ensure_ascii=False, separators=(",", ":")))
    return int(summary["failed"] != 0)


def environment_reason(error):
    # Keep expiry/risk reasons visible. Resource-not-found/permission errors must fail:
    # they may indicate a bad sample or a broken request rather than an environment issue.
    markers = ("登录", "凭据", "过期", "风控", "未登录", "not logged in", "credential expired")
    if any(marker in error for marker in markers):
        return "authentication_or_risk: " + error
    return None


def validate(method, params, reply):
    payload = {key: value for key, value in reply.items() if key not in ("id", "ok")}
    if not payload:
        raise ValueError("success envelope has no payload")
    if method == "fetch_new_albums":
        albums = reply.get("albums")
        if type(reply.get("total")) is not int or not isinstance(albums, list):
            raise ValueError("new albums total/albums schema mismatch")
        if params.get("page", 1) == 1 and not albums:
            raise ValueError("first new-album page is empty")
        for album in albums:
            if not isinstance(album, dict) or not album.get("mid") or not (album.get("title") or album.get("name")):
                raise ValueError("new album lacks MID/title")
            if not isinstance(album.get("singers"), list):
                raise ValueError("new album singers must be a list")
    elif method in ("fetch_singing_annotations", "has_ai_dictionary"):
        key = "exists" if method == "has_ai_dictionary" else "hasSingingAnnotationsLyric"
        if type(reply.get(key)) is not bool:
            raise ValueError(key + " must be a boolean")
    elif method in ("fetch_multi_style_lyrics", "fetch_ai_dictionary"):
        key = "lyrics" if method == "fetch_multi_style_lyrics" else "dictList"
        rows = reply.get(key)
        if not isinstance(rows, list):
            raise ValueError(key + " must be a list (empty is allowed)")
        for row in rows:
            if not isinstance(row, dict):
                raise ValueError(key + " items must be objects")
            fields = ("styleName", "lyric") if key == "lyrics" else (
                "phrase", "explain", "lyricText", "transLyricText", "lyricTimestamp")
            if not all(isinstance(row.get(field), str) for field in fields):
                raise ValueError(key + " item text fields schema mismatch")
            if key == "lyrics" and (type(row.get("style")) is not int or type(row.get("timestamp")) is not int):
                raise ValueError("multi-style lyric style/timestamp schema mismatch")
    elif method == "fetch_user_liked_songs":
        songs = reply.get("songs")
        info = reply.get("info")
        if not isinstance(songs, list) or not isinstance(info, dict):
            raise ValueError("liked songs songs/info schema mismatch")
        if not all(type(reply.get(key)) is int for key in ("size", "total", "hasmore")):
            raise ValueError("liked songs pagination schema mismatch")
        if reply["size"] != len(songs):
            raise ValueError("liked songs decoded count differs from size")
        if any(not isinstance(song, dict) or not song.get("songMid") or not song.get("title") for song in songs):
            raise ValueError("liked song lacks MID/title")
    elif method == "query_songs":
        tracks = reply.get("tracks")
        if not isinstance(tracks, list) or not tracks:
            raise ValueError("known song query returned no tracks")
        if not any(track.get("songMid") == params["songs"][0]["mid"]
                   and isinstance(track.get("title"), str) and track["title"]
                   for track in tracks if isinstance(track, dict)):
            raise ValueError("known song MID/title absent in tracks")
    elif method == "fetch_cdn_dispatch":
        nodes = reply.get("sip")
        if not isinstance(nodes, list) or not nodes or not all(
                isinstance(node, str) and node.startswith(("http://", "https://")) for node in nodes):
            raise ValueError("CDN dispatch lacks usable sip URLs")
    elif method == "fetch_sheet_music":
        if not isinstance(reply.get("result"), list) or not isinstance(reply.get("totalMap"), dict):
            raise ValueError("sheet result/totalMap schema mismatch")
    elif method == "has_sheet_music":
        keys = ("hasGuitar", "hasMore", "hasLdy", "hasQrcx", "hasChongChong")
        if not all(key in reply and (reply[key] is None or isinstance(reply[key], bool)) for key in keys):
            raise ValueError("sheet availability flags schema mismatch")
    elif method == "resolve_song_urls":
        rows = reply.get("data")
        if not isinstance(rows, list) or not rows:
            raise ValueError("song URL response lacks requested item")
        for row in rows:
            if not isinstance(row, dict) or row.get("mid") != params["items"][0]["mid"]:
                raise ValueError("song URL item MID mismatch")
            code = row.get("result")
            if type(code) is not int:
                raise ValueError("song URL item lacks numeric result")
            if code in (104003, 104004, 104013):
                return "playback_entitlement: item result=" + str(code)
            if code != 0:
                raise ValueError("unexpected song URL item result=" + str(code))
            if not isinstance(row.get("purl"), str) or not row["purl"]:
                raise ValueError("successful song URL item lacks purl")
    elif method in ("fetch_other_versions", "fetch_song_producer"):
        if not isinstance(reply.get("data"), list):
            raise ValueError("data must be a list (empty is allowed)")
    elif method in ("general_search", "search_extra"):
        keys = ("searchid", "perpage", "nextpage", "totalNum") if method == "search_extra" else (
            "searchid", "perpage", "nextpage", "nextpageStart")
        if not all(key in reply for key in keys):
            raise ValueError("search metadata missing")
        if not any(reply.get(key) is not None for key in ("searchid", "perpage", "nextpage")):
            raise ValueError("search metadata is entirely null")
        buckets = ("song", "singer", "album", "songlist", "mv", "user", "audioAlum")
        for key in buckets:
            bucket = reply.get(key)
            expected = list if method == "search_extra" else dict
            if bucket is not None and not isinstance(bucket, expected):
                raise ValueError("search bucket " + key + " schema mismatch")
            if isinstance(bucket, dict) and bucket.get("items") is not None and not isinstance(bucket["items"], list):
                raise ValueError("search bucket " + key + " items must be a list")
        # A popular first-page query should not silently decode into an all-empty payload.
        if method == "general_search" and params.get("page", 1) == 1:
            if not any(isinstance(reply.get(key), dict) and reply[key].get("items") for key in buckets):
                raise ValueError("popular general search returned no decoded items")
    return None


def cases_for(method):
    base = dict(PARAMS[method])
    if method == "search_extra":
        for search_type in SEARCH_TYPES:
            yield "type=" + str(search_type) + ",page=1", {**base, "searchType": search_type}
        yield "type=0,page=2", {**base, "searchType": 0, "page": 2}
    elif method == "fetch_sheet_music":
        for sheet_type in (0, 1, 2):
            yield "type=" + str(sheet_type), {**base, "ttype": sheet_type}
    elif method in ("fetch_multi_style_lyrics", "fetch_ai_dictionary"):
        yield "reference-sample", base
        yield "no-extra-content-sample", {"songId": 2314161}
    else:
        yield "page=1" if "page" in base else "default", base
        if method in PAGE_TWO:
            yield "page=2", {**base, "page": 2}


def main(argv):
    summary = result()
    env = dict(os.environ)
    if argv and Path(argv[0]).is_dir():
        env["QQMUSIC_HELPER_NEXT_DIR"] = argv.pop(0)
    else:
        env.pop("QQMUSIC_HELPER_NEXT_DIR", None)
    selected = list(dict.fromkeys(argv or PARAMS))
    rejected = [method for method in selected if method not in PARAMS]
    if rejected:
        for method in rejected:
            record(summary, method, "allowlist", "failed", "method is not in the explicit read-only allowlist")
        return finish(summary)  # Reject the entire invocation before a build or dispatch.
    try:
        timeout = float(os.environ.get("PORT_SMOKE_TIMEOUT_SECONDS", "30"))
        if not math.isfinite(timeout) or not 1 <= timeout <= 120:
            raise ValueError("timeout must be between 1 and 120 seconds")
        if os.environ.get("PORT_SMOKE_SKIP_BUILD") != "1":
            build = subprocess.run(["cargo", "build", "--bin", "qqmusic-helper-next"], cwd=ROOT,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=180)
            if build.returncode:
                raise ValueError("binary build failed")
        binary = ROOT / "target/debug/qqmusic-helper-next"
        if not os.access(binary, os.X_OK):
            raise ValueError("helper binary is missing or not executable")
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        record(summary, "setup", "setup", "failed", str(error))
        return finish(summary)
    for method in selected:
        summary["checked"].append(method)
        for case, params in cases_for(method):
            if method == "fetch_user_liked_songs":
                euin = env.get("PORT_SMOKE_EUIN", "").strip()
                if not euin:
                    reason = "target_not_configured: set PORT_SMOKE_EUIN to the target user's encrypted UIN"
                    record(summary, method, case, "skipped", reason)
                    print(method + " [" + case + "]: skipped — " + reason, file=sys.stderr)
                    continue
                params = {**params, "euin": euin}
            started = time.monotonic()
            status, reason = "failed", "unknown error"
            try:
                request = {"id": "smoke", "method": method, "params": params}
                process = subprocess.run([str(binary)], input=json.dumps(request) + "\n", env=env,
                                         capture_output=True, text=True, timeout=timeout)
                if process.returncode:
                    raise ValueError("helper exited with code " + str(process.returncode))
                lines = process.stdout.splitlines()
                if len(lines) != 1:
                    raise ValueError("expected exactly one JSON reply, got " + str(len(lines)))
                reply = json.loads(lines[0])
                if not isinstance(reply, dict) or reply.get("id") != "smoke" or type(reply.get("ok")) is not bool:
                    raise ValueError("invalid response envelope/id/ok")
                if reply["ok"]:
                    skipped_reason = validate(method, params, reply)
                    status, reason = ("skipped", skipped_reason) if skipped_reason else ("passed", "schema/content validated")
                else:
                    error = reply.get("error")
                    if not isinstance(error, str) or not error:
                        raise ValueError("failed envelope lacks error text")
                    skipped_reason = environment_reason(error)
                    status, reason = ("skipped", skipped_reason) if skipped_reason else ("failed", error)
            except subprocess.TimeoutExpired:
                reason = "helper call exceeded " + str(timeout) + " seconds"
            except (OSError, ValueError) as error:
                reason = str(error)
            duration = round((time.monotonic() - started) * 1000)
            record(summary, method, case, status, reason, duration)
            print(method + " [" + case + "]: " + status + " — " + reason, file=sys.stderr)
    return finish(summary)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
