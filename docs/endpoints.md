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

> ⏳ 的条目形状已经确定（见上表），实现方式是同一套：`methods.rs` 里加一个分支 + `api.rs` 里加一个
> `#[export]` 包装 + 一条真账号的验证。`docs/parsing.md` 记着每类响应里那些不直观的地方。

## 响应字段的解析入口

曲目统一由 `methods::decode_track` 映射（`songId/songMid/mediaMid/title/artist/album/albumMid/albumId/
imageURL/duration/payPlay/singerMid/singers`）。上游同一实体的键名在不同接口里不一样
（`mid`/`songmid`/`albummid`、`singer` 有时是列表有时是字符串），取值工具在 `upstream.rs`：
`first_text` / `first_int` / `first_object` / `first_array` 按候选键依次尝试。
