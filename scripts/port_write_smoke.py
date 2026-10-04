#!/usr/bin/env python3
"""Explicit, reversible real-account tests. No login flow or screenshots.

Run after cargo build --bin qqmusic-helper-next:
  python3 scripts/port_write_smoke.py --execute-writes --report docs/write-smoke.json
Only newly added memberships and a uniquely named test playlist are modified.
Existing liked/favourite/dislike entries are never removed for a test.
"""
import argparse
import datetime
import json
import pathlib
import selectors
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
MID = "003w2xz20QlUZt"


class Session:
    def __init__(self, audit):
        self.audit = audit
        self.sequence = 0
        self.process = subprocess.Popen(
            [str(ROOT / "target/debug/qqmusic-helper-next")],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, bufsize=1,
        )
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)

    def call(self, method, params=None, write=False):
        self.sequence += 1
        reqid = str(self.sequence)
        self.process.stdin.write(json.dumps({"id": reqid, "method": method, "params": params or {}}) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + 45
        while True:
            if not self.selector.select(max(0, deadline - time.monotonic())):
                if write:
                    self.audit["writes"].append({"method": method, "status": "unknown", "reason": "timeout; inspect remote state"})
                raise RuntimeError(f"{method}: timeout")
            value = json.loads(self.process.stdout.readline())
            if value.get("id") == reqid:
                break
            # A timed-out request may reply while compensating a prior write.
            # It must not consume the response belonging to the cleanup call.
            if time.monotonic() >= deadline:
                raise RuntimeError(f"{method}: response correlation timeout")
        if write:
            self.audit["writes"].append({"method": method, "status": "responded" if value.get("ok") else ("unknown" if "网络" in value.get("error", "") else "rejected"), "error": value.get("error")})
        if not value.get("ok"):
            raise RuntimeError(f"{method}: {value.get('error', 'unknown upstream error')}")
        return value

    def close(self):
        self.selector.close()
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.terminate()


def require(condition, reason):
    if not condition:
        raise RuntimeError(reason)


def memberships(session, method, key, id_key="id"):
    ids = set()
    for page in range(1, 31):
        value = session.call(method, {"page": page, "limit": 100, "num": 100})
        rows = value.get(key)
        require(isinstance(rows, list), f"{method}: missing {key}")
        ids.update(int(row[id_key]) for row in rows if row.get(id_key) is not None)
        if not value.get("hasmore") and not value.get("hasMore"):
            require(value.get("total") is None or len(ids) >= int(value["total"]), f"{method}: incomplete membership snapshot")
            return ids
    raise RuntimeError(f"{method}: exceeded snapshot page bound")


def likes(session):
    ids = set()
    for page in range(1, 101):
        data = session.call("fetch_liked_songs", {"page": page, "limit": 100})["likedSongs"]
        rows = data["tracks"]
        ids.update(int(row["songId"]) for row in rows)
        if len(ids) >= int(data["total"]):
            return ids
        if not rows:
            return ids  # Upstream total can include unavailable/deleted songs.
    raise RuntimeError("liked songs snapshot exceeds bound")


def dislikes(session):
    ids = set()
    last_id = 0
    for page in range(1, 51):
        value = session.call("fetch_dislike_list", {"page": page, "cmd": 3, "lastid": last_id})
        require(value.get("retcode") == 0, "dislike list business failure")
        rows = value["songs"]
        if not rows:
            return ids
        new_ids = {int(row["id"]) for row in rows}
        require(bool(new_ids - ids), "dislike cursor did not advance")
        ids.update(new_ids)
        last_id = int(rows[-1]["id"])
    raise RuntimeError("dislike snapshot exceeds bound")


def check_group(audit, name, test):
    try:
        result = test()
        if result is not False:
            audit["groups"].append({"name": name, "status": "passed"})
    except Exception as error:
        audit["groups"].append({"name": name, "status": "failed", "reason": str(error)})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute-writes", action="store_true", help="explicitly enable real account writes")
    parser.add_argument("--report", type=pathlib.Path, required=True)
    args = parser.parse_args()
    if not args.execute_writes:
        parser.error("--execute-writes is required; no remote call made")
    audit = {"startedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(), "writes": [], "groups": [], "restorations": [], "excluded": ["login/QR flows", "clear_dislike_songs: existing dislike entries retain their original timestamps and order"], "screenshots": 0}
    session = Session(audit)
    try:
        # A public VIP response does not establish an authenticated session.
        baseline_albums = memberships(session, "fetch_fav_albums", "albums")
        baseline_playlists = memberships(session, "fetch_fav_playlists", "playlists")
        baseline_likes = likes(session)
        baseline_dislikes = dislikes(session)
        before_created = session.call("fetch_created_playlists")["playlists"]
        before_created_ids = {int(x["dirid"]) for x in before_created}
        query = session.call("query_songs", {"songs": [{"mid": MID}]})["tracks"]
        require(len(query) == 1, "known song lookup failed")
        song_id, album_id = int(query[0]["songId"]), int(query[0]["albumId"])
        audit["targets"] = {"songId": song_id, "albumId": album_id}

        def playlist_test():
            name = "HelperNext接口测试-" + datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d-%H%M%S")
            dirid, tid, comment_id = None, None, None
            try:
                created = session.call("create_playlist", {"dirName": name}, write=True)
                require(created.get("retCode") == 0, "playlist creation business failure")
                new_dirid, new_tid = int(created["dirid"]), int(created["resultId"])
                require(new_dirid > 0 and new_tid > 0 and new_dirid not in before_created_ids, "creation did not identify a new playlist")
                dirid, tid = new_dirid, new_tid
                audit["temporaryPlaylist"] = {"name": name, "dirid": dirid, "id": tid}
                rows = session.call("fetch_created_playlists")["playlists"]
                require(any(int(x["dirid"]) == dirid for x in rows), "new playlist missing from readback")
                params = {"dirId": dirid, "tid": tid, "songInfo": [{"songId": song_id, "songType": 0}]}
                for method in ["add_playlist_songs", "add_playlist_songs"]:
                    require(session.call(method, params, write=True).get("success") is True, "adding a song failed")
                tracks = session.call("fetch_playlist_tracks", {"songlistId": tid})["tracks"]
                require(sum(int(x["songId"]) == song_id for x in tracks) == 1, "duplicate add did not preserve one membership")
                for method in ["remove_playlist_songs", "remove_playlist_songs"]:
                    require(session.call(method, params, write=True).get("success") is True, "removing a song failed")
                require(not session.call("fetch_playlist_tracks", {"songlistId": tid})["tracks"], "test playlist did not become empty")
                added = session.call("add_comment", {"bizType": 3, "bizId": tid, "content": "HelperNext接口连通性测试，完成后删除。"}, write=True)
                comment_id = added.get("resultId")
                require(added.get("subcode") == 0 and comment_id, "comment creation rejected or did not return an ID")
                comment_params = {"bizType": 3, "bizId": tid, "page": 1, "limit": 10}
                def visible_comment():
                    rows = session.call("fetch_new_comments", comment_params).get("comments")
                    require(isinstance(rows, list), "comment readback lacks a list")
                    return any(str(row.get("cmid")) == str(comment_id) for row in rows)
                before_visible = False
                for attempt in range(3):
                    before_visible = visible_comment()
                    if before_visible:
                        break
                    time.sleep(1)
                require(before_visible, "added comment not visible in readback")
                require(session.call("delete_comment", {"cmId": comment_id}, write=True).get("deleted") is True, "comment deletion rejected")
                after_visible = True
                for attempt in range(3):
                    after_visible = visible_comment()
                    if not after_visible:
                        break
                    time.sleep(1)
                require(not after_visible, "comment remains in readback after deletion")
                comment_id = None
                audit["restorations"].append({"resource": "temporary comment", "verified": True, "evidence": "comment visible before delete, absent after delete; parent playlist also removed"})
            finally:
                try:
                    if comment_id:
                        require(session.call("delete_comment", {"cmId": comment_id}, write=True).get("deleted") is True, "comment cleanup rejected")
                finally:
                    # Even a failed/unparseable create reply may have created the
                    # resource. Resolve only this run's exact unique name.
                    if dirid is None:
                        rows = session.call("fetch_created_playlists")["playlists"]
                        matches = [x for x in rows if x.get("title") == name and int(x["dirid"]) not in before_created_ids]
                        require(len(matches) <= 1, "ambiguous newly created test playlist")
                        if matches:
                            dirid = int(matches[0]["dirid"])
                    if dirid:
                        cleanup_error = None
                        try:
                            deleted = session.call("delete_playlist", {"dirId": dirid}, write=True)
                            require(deleted.get("retCode") == 0, "playlist cleanup rejected")
                        except Exception as error:
                            cleanup_error = str(error)
                        rows = session.call("fetch_created_playlists")["playlists"]
                        after = {int(x["dirid"]) for x in rows}
                        require(after == before_created_ids, "created playlist membership differs after cleanup")
                        audit["restorations"].append({"resource": "temporary playlist and its songs", "verified": True, "evidence": "created playlist directory IDs equal baseline", "responseError": cleanup_error})

        check_group(audit, "playlist create/add/deduplicate/remove/comment/delete", playlist_test)

        def album_test():
            if album_id in baseline_albums:
                audit["groups"].append({"name": "album roundtrip", "status": "skipped", "reason": "candidate was already favoured; original order retained"})
                return False
            try:
                require(session.call("fav_album", {"albumIds": [album_id]}, write=True).get("success") is True, "album favourite failed")
                require(album_id in memberships(session, "fetch_fav_albums", "albums"), "album not in readback")
            finally:
                require(session.call("unfav_album", {"albumIds": [album_id]}, write=True).get("success") is True, "album cleanup rejected")
                require(memberships(session, "fetch_fav_albums", "albums") == baseline_albums, "album membership differs from baseline")
                audit["restorations"].append({"resource": "album favourites", "verified": True})
        check_group(audit, "album roundtrip", album_test)

        def liked_test():
            if song_id in baseline_likes:
                audit["groups"].append({"name": "liked song roundtrip", "status": "skipped", "reason": "candidate already liked; original order retained"})
                return False
            params = {"songId": song_id, "songType": 0}
            try:
                session.call("set_liked", {**params, "liked": True}, write=True)
                require(song_id in likes(session), "song missing from likes readback")
            finally:
                session.call("set_liked", {**params, "liked": False}, write=True)
                require(likes(session) == baseline_likes, "liked memberships differ after cleanup")
                audit["restorations"].append({"resource": "liked songs", "verified": True})
        check_group(audit, "liked song roundtrip", liked_test)

        def dislike_test():
            if song_id in baseline_dislikes:
                audit["groups"].append({"name": "dislike roundtrip", "status": "skipped", "reason": "candidate already disliked; original metadata retained"})
                return False
            params = {"idType": 1, "values": [song_id]}
            try:
                require(session.call("add_dislike", params, write=True).get("success") is True, "dislike add rejected")
                require(song_id in dislikes(session), "dislike membership not visible")
            finally:
                require(session.call("cancel_dislike", params, write=True).get("success") is True, "dislike cleanup rejected")
                require(dislikes(session) == baseline_dislikes, "dislike memberships differ after cleanup")
                audit["restorations"].append({"resource": "disliked songs", "verified": True})
        check_group(audit, "dislike roundtrip", dislike_test)

        def playlist_fav_test():
            result = session.call("search_playlists", {"keyword": "周杰伦", "limit": 10})
            rows = result.get("playlists", [])
            candidates = [int(x["id"]) for x in rows if x.get("id") and int(x["id"]) not in baseline_playlists]
            require(candidates, "no unfavoured public playlist candidate")
            pid = candidates[0]
            audit["targets"]["externalPlaylistId"] = pid
            try:
                require(session.call("fav_playlist", {"playlistId": pid}, write=True).get("success") is True, "playlist favourite failed")
                require(pid in memberships(session, "fetch_fav_playlists", "playlists"), "playlist not in favourites readback")
            finally:
                require(session.call("unfav_playlist", {"playlistId": pid}, write=True).get("success") is True, "playlist favourite cleanup rejected")
                require(memberships(session, "fetch_fav_playlists", "playlists") == baseline_playlists, "playlist favourites differ after cleanup")
                audit["restorations"].append({"resource": "external playlist favourites", "verified": True})
        check_group(audit, "external playlist favourite roundtrip", playlist_fav_test)
    except Exception as error:
        audit["fatal"] = str(error)
    finally:
        session.close()
        audit["finishedAt"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(audit, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(audit, ensure_ascii=False))
    return 1 if "fatal" in audit or any(x["status"] == "failed" for x in audit["groups"]) else 0


if __name__ == "__main__":
    raise SystemExit(main())
