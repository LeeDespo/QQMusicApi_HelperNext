# 接口清单

每个"能力"对应的上游请求形状，以及在本组件里的实现状态。
每条都是**实测**出来的：接口形状来自 QQMusicApi 的源码（`modules/*.py` 的 `_build_cgi` / `_build_http` 调用），
状态列以真账号跑通为准。

## 两个上游

| 上游 | 形状 | 用途 |
|---|---|---|
| `POST https://u.y.qq.com/cgi-bin/musicu.fcg` | `{comm:{…}, req_0:{module, method, param}}`，支持 `req_0..req_N` 批量 | 目录、账号列表、歌词、取流、登录 |
| `GET https://c.y.qq.com/fav/fcgi-bin/fcg_get_profile_order_asset.fcg` | 表单式 query：`ct=20&cid=205360956&userid=<数字uin>&reqtype=<2=专辑,3=歌单>&sin=&ein=` | **只有它**能取账号自己的歌单与收藏专辑 |

`comm` 的平台档案必须用 **web**：`cv 4747474 / ct 24 / platform "yqq.json" / needNewCode 1`，
外加 `uin` 与 `g_tk = hash33(qm_keyst)`；授权靠 Cookie（`uin`、`qm_keyst`）。
用库默认档案请求账号列表会被拒（`10004`）。

## 实测状态（2026-09-30，真账号）

实现落地后逐条跑过一遍，如实记录——**通过**的可以直接用，**待修**的已经知道要查什么：

| 能力 | 第一次实测 | 结论 |
|---|---|---|
| `fetch_song_detail` | ✅ 不遗憾 / 简介 176 字 | 通过（web 档案即可） |
| `fetch_radio_stations` | ✅ 11 组，首组「猜你喜欢」 | 通过 |
| `fetch_new_songs` | ✅ 57 首，首条「回响」 | 通过 |
| `resolve_song_url` | ✅ playable=true，quality=128，拿到 CDN 地址 | 音质阶梯与文件名构造正确 |
| `fetch_album_tracks` | ❌ 0 条 | 响应键名或参数待修：先 dump 原始响应比对 |
| `fetch_artist_songs` / `fetch_artist_albums` | ❌ 0 条 | 同上（同一参数的 Python 实现能取到，说明是键名/参数细节） |
| `fetch_toplist_categories` | ❌ 0 组 | 同上（`GetAll` 的分组键名待确认） |
| `fetch_lyric` | ❌ 空 | 四个 base64 字段一个都没解出来 → 键名或 `qrc/crypt` 组合待确认 |
| `fetch_recommend_feed` | ❌ 0 首 | `get_radio_track` 的参数/响应键待确认 |
| `fetch_artist_detail` | ❌ `10006` | 参数形状不对（`singer_mids` 的写法或需换 `GetSingerDetail` 之外的接口） |
| `search_*`（四类） | 未实现 | web 与 android 两个档案都试过：`code 0` 但 `meta.sum = 0`，是**查询形状**问题（很可能缺真实 qimei 等设备参数），不是档案问题 |
| `set_liked` / 扫码登录 | 未实现 | 形状已在下方列出 |

**排查方法（下一步直接照做）**：同一个调用用 Python 版 helper 跑一遍并把原始响应落盘，
与本组件的响应逐键对比——两者的 module/method/参数一致，差别只可能在
① 响应键名（本组件用候选键取值，可能全都落空）② 平台档案 ③ 参数里被上游视为必需而我漏掉的字段。
`docs/parsing.md` 里"字段名在不同接口里不同"那条是同一类问题的记录。

## 状态

| 能力 | 方法名 | 上游 module / method | 参数要点 | 状态 |
|---|---|---|---|---|
| 组件信息 | `get_helper_info` | — | — | ✅ |
| 登录状态 | `get_login_status` | `music.UserInfo.userInfoServer` / `GetLoginUserInfo` | `{}` | ✅ 昵称在 `info.nick` |
| 导入登录（网页 cookie） | `import_cookies` | — | `uin` + `qm_keyst` 即可 | ✅ |
| 退出登录 | `logout` | — | — | ✅ |
| 我喜欢 | `fetch_liked_songs` | `music.srfDissInfo.DissInfo` / `CgiGetDiss` | `disstid=0, dirid=201, song_begin, song_num, tag, userinfo, orderlist` | ✅ 总数在 `dirinfo.songnum` |
| 歌单/排行榜曲目 | `fetch_playlist_tracks` | 同上 | `disstid=<id>` + `song_begin/song_num` | ✅ 同上 |
| 我的歌单 | `fetch_user_playlists` | 老 fcgi | `reqtype=3` | ✅ 保留目录（无 `dissid`）跳过 |
| 收藏专辑 | `fetch_liked_albums` | 老 fcgi | `reqtype=2` | ✅ `pubtime` 是时间戳 |
| 关注的歌手 | `fetch_followed_artists` | `music.concern.RelationList` / `GetFollowSingerList` | `HostUin=<encrypt_uin>, From, Size` | ✅ 返回键是 `List`（大写） |
| 限流/熔断状态 | `get_status` | — | — | ✅ |
| 专辑曲目 | `fetch_album_tracks` | `music.musichallAlbum.AlbumSongList` / `GetAlbumSongList` | `albumMid`, `begin`, `num`, `order` | ⏳ |
| 歌手歌曲 | `fetch_artist_songs` | `musichall.song_list_server` / `GetSingerSongList` | `singerMid, order=1, number, begin` | ⏳ "最新"需本地按 `time_public` 排序 |
| 歌手专辑 | `fetch_artist_albums` | `music.musichallAlbum.AlbumListServer` / `GetAlbumList` | `singerMid, order=1, number, begin` | ⏳ 同上 |
| 歌手资料/简介 | `fetch_artist_detail` | `music.musichallSinger.SingerInfoInter` / `GetSingerDetail` | `singer_mids, ex_singer, wiki_singer, …` | ⏳ |
| 排行榜分组 | `fetch_toplist_categories` | `music.musicToplist.Toplist` / `GetAll` | `{}` | ⏳ |
| 排行榜曲目 | `fetch_toplist_tracks` | `music.musicToplist.Toplist` / `GetDetail` | `topId, offset, num` | ⏳ 曲目在 `songInfoList`，总数 `totalNum` |
| 新歌 | `fetch_new_songs` | `newalbum.NewAlbumServer` / `get_new_album_info` | `area, num, start` | ⏳ |
| 搜索（歌曲/歌手/专辑/歌单） | `search_songs` / `search_artists` / `search_albums` / `search_playlists` | `client.search.search_by_type`（`SearchType` 0/1/2/3） | `keyword, num, page, highlight=false` | ⏳ 四类共用一个端点 |
| 猜你喜欢 | `fetch_recommend_feed` | 库 `recommend.get_guess_recommend` | — | ⏳ |
| 歌曲简介 | `fetch_song_detail` | `music.pf_song_detail_svr` / `get_song_detail_yqq` | `song_mid` | ⏳ 文本在 `info.intro.content[].value` |
| 专辑简介 | `fetch_album_detail` | `music.musichallAlbum.AlbumInfoServer` / `GetAlbumDetail` | `albumMid` | ⏳ |
| 歌词 | `fetch_lyric` | `music.musichallSong.PlayLyricInfo` / `GetPlayLyricInfo` | `songMID, songID=0, format=json, crypt=0, qrc=0, trans, roma` | ⏳ 逐字在 `qrc` 的 base64 里 |
| 取流地址 | `resolve_song_url` | `music.vkey.GetVkey` / `GetCdnDispatch` 等 | `songMid, mediaMid, 各档文件名 M500/M800/F000/C400` | ⏳ 需按音质阶梯探测并判读 per-file result 码 |
| 电台分组 / 曲目 | `fetch_radio_*` | 库 `radio` 模块 | — | ⏳ |
| 收藏 / 取消收藏（唯一的写） | `set_liked` | `music.musicasset.PlaylistBaseWrite` / `AddSonglist`·`DelSonglist` | `dirid=201`, `songIds/v_songIds` | ⏳ |
| 扫码登录 | `start_login` / `poll_login` | `music.login.LoginServer` / `CreateQRCode` + `Login` | `tmeAppID=qqmusic`；轮询用 `musicid, qrCodeID, token` | ⏳ 另有 QQ Connect 与微信两条路径 |

> ⏳ 的条目形状已经确定（见上表与下面的"精确形状"），实现方式是同一套：`methods.rs` 里加一个分支 +
> `api.rs` 里加一个 `#[export]` 包装 + 一条真账号的验证。`docs/parsing.md` 记着每类响应里那些不直观的地方。

## 精确形状（已从 QQMusicApi 源码逐条核对，可直接照写）

### 歌曲 / 专辑资料

```
fetch_song_detail   music.pf_song_detail_svr / get_song_detail_yqq   {"song_mid": mid}
                    → info.intro.content[].value（多段用换行连接）；另有 info.company/genre/lan/pub_time
fetch_album_detail  music.musichallAlbum.AlbumInfoServer / GetAlbumDetail
                    {"albumId": <数字>} 或 {"albumMId": <mid>}（二选一，按传入的是数字还是 mid）
fetch_album_tracks  music.musichallAlbum.AlbumSongList / GetAlbumSongList
                    {"albumId": <数字>} 或 {"albumMid": <mid>} + {"begin": offset, "num": limit, "order": 0}
```

### 歌手

```
fetch_artist_songs   musichall.song_list_server / GetSingerSongList
                     {"singerMid": mid, "order": 1, "number": num, "begin": (page-1)*num}
                     "最新" 上游不支持排序 → 本地按每首的 album.time_public 倒序
fetch_artist_albums  music.musichallAlbum.AlbumListServer / GetAlbumList
                     同上参数；"最新" 同理按 album time_public 倒序
fetch_artist_detail  music.musichallSinger.SingerInfoInter / GetSingerDetail
                     {"singer_mids": [mid], "ex_singer": true, "wiki_singer": true,
                      "group_singer": true, "pic": true, "photos": true}
                     简介取 wiki/desc 字段；头像取 pic 相关字段
```

### 排行榜

```
fetch_toplist_categories  music.musicToplist.Toplist / GetAll       {}
                          → data 里的分组（每组含 topList）与榜单条目（topId/topTitle/cover）
fetch_toplist_tracks      music.musicToplist.Toplist / GetDetail
                          {"topId": id, "offset": n, "num": m}[, {"withTags": true}]
                          曲目在 songInfoList（不是 data.data.song），总数在 totalNum
```

### 电台 / 新歌 / 猜你喜欢

```
fetch_radio_stations  pf.radiosvr / GetRadiolist        {"uin": <数字uin 或 "0">}
                      → radio_list[].list[]（分组）{id, title, pic_url}
fetch_radio_tracks    pf.radiosvr / GetRadiosonglist
                      {"id": stationId, "firstplay": 1|0, "num": limit}
                      → 曲目在 data.track_list / songlist
fetch_new_songs       newsong.NewSongServer / get_new_song_info   {"type": <地区码>}
fetch_guess_recommend music.radioProxy.MbTrackRadioSvr / get_radio_track
                      {"id": 99, "num": 5, "from": 0, "scene": 0, "song_ids": []}（需凭据）
```

### 歌词 / 取流

```
fetch_lyric      music.musichallSong.PlayLyricInfo / GetPlayLyricInfo
                 {"songMID": mid, "songID": id?, "format": "json", "crypt": 0,
                  "qrc": 1, "trans": 1, "roma": 1}
                 lyric/trans/roma/qrc 都是 base64；逐字时间在 qrc 里
resolve_song_url music.vkey.GetVkey / UrlGetVkey（加密档位时是 music.vkey.GetEVkey / CgiGetEVkey）
                 {"uin": <数字uin>, "filename": [...], "guid": <随机>, "songmid": [...],
                  "songtype": [...], "ctx": 0}
                 文件名构造：有 media_mid 用 `<档位前缀><media_mid><后缀>`（M500.mp3 / M800.mp3 /
                 F000.flac / C400.m4a），没有 media_mid 时用 `<前缀><mid><mid><后缀>`
                 逐档探测并读 midurlinfo[].result（0 成功 / 104003 无权限 / 104004 取票失败 /
                 104013 设备受限），返回第一个可用的 purl + vkey
```

### 搜索（四类共用）

```
search_songs / search_artists / search_albums / search_playlists
music.search.SearchCgiService / DoSearchForQQMusicMobile
{"searchid": <随机>, "query": keyword, "search_type": 0|1|2|3,
 "num_per_page": num, "page_num": page, "highlight": false, "grp": true,
 "selectors": {}, "vec_selectors": []}
→ 结果在 body.item_song / body.singer / body.item_album / body.item_songlist；总数 meta.sum

⚠ 这一条 QQMusicApi 明确用 **Platform.ANDROID**（ct 11 / cv 14090008，带 qimei 等设备参数），
而本组件目前全局用 web 档案。实现搜索前要先给 `upstream.rs` 加"按调用选平台档案"的能力，
并用真账号确认 web 档案是否也能过——**这是剩余工作里唯一的技术未知项**。
```

### 收藏 / 取消收藏（唯一的写）

```
set_liked  music.musicasset.PlaylistDetailWrite / AddSonglist（收藏）· DelSonglist（取消）
           {"dirId": 201, "tid": 0, "bFmtUtf8": true,
            "v_songInfo": [{"songId": <数字>, "songType": 0}]}
           需要 songId（数字）；只给 songMid 时要先用 song.query_song 换 songId
```

### 扫码登录（多步）

```
start_login  music.login.LoginServer / CreateQRCode    {"tmeAppID": "qqmusic", <版本参数>}
             → qrCodeID/token/二维码内容（另有 QQ Connect 与微信两条路径，各自走不同授权域名）
poll_login   music.login.LoginServer / Login
             {"musicid": <数字>, "qrCodeID": ..., "token": ...}
             轮询事件：SCAN / CONF / DONE / TIMEOUT / REFUSE；DONE 时响应里带回凭据
             → 等价于 import_credential(uin, qm_keyst)
```

### 后续实现顺序（建议）

1. 资料类（歌曲/专辑简介、专辑曲目、歌手三项）—— web 档案，风险最低
2. 列表类（榜单、电台、新歌、猜你喜欢）—— web 档案
3. 歌词 + 取流 —— 需要 media_mid 与音质阶梯
4. 搜索四类 —— **先解决平台档案**（见上）
5. 收藏写（需要 songId 换算）+ 扫码登录（多步状态机）

## 响应字段的解析入口

曲目统一由 `methods::decode_track` 映射（`songId/songMid/mediaMid/title/artist/album/albumMid/albumId/
imageURL/duration/payPlay/singerMid/singers`）。上游同一实体的键名在不同接口里不一样
（`mid`/`songmid`/`albummid`、`singer` 有时是列表有时是字符串），取值工具在 `upstream.rs`：
`first_text` / `first_int` / `first_object` / `first_array` 按候选键依次尝试。
