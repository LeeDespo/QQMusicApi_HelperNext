# 接口清单

组件对外暴露的每一个方法，以及它背后打的上游接口。**这一份是参考手册，不是开发记录**：
状态列写的是当前实现，不记日期、不记排查过程；那些坑记在[解析要点](parsing.md)里。

## 一、上游与请求信封

| 上游 | 形状 | 用途 |
|---|---|---|
| `POST https://u.y.qq.com/cgi-bin/musicu.fcg` | `{comm:{…}, req_0:{module, method, param}}` | 目录、账号列表、取流、登录 |
| `GET https://c.y.qq.com/fav/fcgi-bin/fcg_get_profile_order_asset.fcg` | 表单式 query，`userid` 用数字 uin，`reqtype` 2=专辑 3=歌单 | 账号自己的歌单与收藏专辑（**只有它**能取） |
| `GET https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg` | 表单式 query，`nobase64=1` | 明文歌词 |
| `ssl.ptlogin2.qq.com` · `graph.qq.com` | 扫码登录的五步握手 | 登录 |

**`comm` 的平台档案按接口分类选择**，选错不报错、只回空数据：

- **web 档案** —— 账号列表（我喜欢 / 我的歌单 / 收藏专辑 / 关注歌手）、曲库详情、榜单、电台、新歌。
  用库默认档案请求账号列表会被拒（`10004`）。
- **android 档案** —— 搜索、取流、收藏（写）、推荐流、歌手资料、封面匹配。
  这些接口要**设备身份**与**设备会话**，两者都在 android 那套 `comm` 里。

## 二、方法清单

除非另有说明，每条都在真实账号上跑通过。

| 能力 | 方法 | 上游 module / method | 参数要点 | 关键回值 |
|---|---|---|---|---|
| 组件信息 | `get_helper_info` | — | — | `helperVersion` / `protocolVersion` / `methods` |
| 限流与熔断状态 | `get_status` | — | — | `status.rateLimit.config` / `status.breakerConfig` |
| 限流配置 | `set_rate_limit` | — | `enabled, windowSeconds, maxRequests` | clamp 到 1–3600 秒 / 1–100000 次 |
| 熔断配置 | `set_breaker` | — | `enabled, failureThreshold, failureWindowSeconds, openSeconds` | 改配置会同时清空熔断状态 |
| 登录状态 | `get_login_status` | `music.UserInfo.userInfoServer` / `GetLoginUserInfo` | `{}` | 昵称在 `info.nick` |
| 导入登录 | `import_cookies` | — | `uin` + `qm_keyst` 即可 | 不需要别的 cookie |
| 退出登录 | `logout` | — | — | 删除凭据文件 |
| 扫码登录 | `start_login` / `poll_login` | `ssl.ptlogin2.qq.com` 五步握手 | `loginType` / `identifier` | `qrcode.imageBase64` / `event` |
| 我喜欢 | `fetch_liked_songs` | `music.srfDissInfo.DissInfo` / `CgiGetDiss` | `dirid=201, song_begin, song_num` | 曲目在 `songlist`，总数在 `dirinfo.songnum` |
| 歌单 / 排行榜曲目 | `fetch_playlist_tracks` | 同上 | `disstid=<id>` + `page` 或 `offset`，`song_num` | 同上 |
| 排行榜分组 | `fetch_toplist_categories` | `music.musicToplist.Toplist` / `GetAll` | `{}` | 组内键名是 `toplist`（小写 l） |
| 排行榜曲目 | `fetch_toplist_tracks` | `music.musicToplist.Toplist` / `GetDetail` | `topId, offset, num` | 曲目在 `songInfoList`，总数 `totalNum` |
| 我的歌单 | `fetch_user_playlists` | 老 fcgi | `reqtype=3` | 无 `dissid` 的保留目录要跳过 |
| 收藏专辑 | `fetch_liked_albums` | 老 fcgi | `reqtype=2` | `pubtime` 是北京时间零点 |
| 关注的歌手 | `fetch_followed_artists` | `music.concern.RelationList` / `GetFollowSingerList` | `HostUin`（数字 uin 或 `encrypt_uin`）, `From`, `Size` | 列表键是 `List`（大写） |
| 歌曲详情 / 简介 | `fetch_song_detail` | `music.pf_song_detail_svr` / `get_song_detail_yqq` | `song_mid`；只给名字时组件先搜索解析 | 简介在 `info.intro.content[].value`，**空即答案** |
| 专辑详情 | `fetch_album_detail` | `music.musichallAlbum.AlbumInfoServer` / `GetAlbumDetail` | `albumMId` 或 `albumId`；也可只给名字 | — |
| 歌手资料 | `fetch_artist_detail` | `music.UnifiedHomepage.UnifiedHomepageSrv` / `GetHomepageHeader` | `SingerMid`；也可只给名字 | 空壳资料报错，不伪装成「未知歌手」 |
| 歌手简介 | `fetch_artist_biography` | 同上 | 同上 | 回答在 `artistDetail` |
| 专辑曲目 | `fetch_album_tracks` | `music.musichallAlbum.AlbumSongList` / `GetAlbumSongList` | `albumMid` 或 `albumId`, `begin`, `num` | 列表键是 `songList`（大写 L），总数在 `totalNum` |
| 歌手歌曲 | `fetch_artist_songs` | `musichall.song_list_server` / `GetSingerSongList` | `singerMid, order=1, number, begin` | 「最新」由组件按 `time_public` 排序 |
| 歌手专辑 | `fetch_artist_albums` | `music.musichallAlbum.AlbumListServer` / `GetAlbumList` | 同上 | `albumID`（大写 ID）是上游的拼写 |
| 电台分组 | `fetch_radio_stations` | `pf.radiosvr` / `GetRadiolist` | `uin` | — |
| 电台曲目 | `fetch_radio_tracks` | `pf.radiosvr` / `GetRadiosonglist` | `id`, `num`, `firstPlay` | 电台是无穷列表 |
| 新歌 | `fetch_new_songs` | `newsong.NewSongServer` / `get_new_song_info` | `type`（地区）, `num`, `start` | — |
| 猜你喜欢 | `fetch_recommend_feed` | `music.radioProxy.MbTrackRadioSvr` / `get_radio_track` | `id=99, num, from, scene` | 需要设备会话 |
| 歌词（整行） | `fetch_lyric` | 明文 fcgi | `songmid`, `nobase64=1` | `lyric` 是明文 LRC |
| 歌词（逐字） | `fetch_lyric` 的 `wordLyric` | `music.musichallSong.PlayLyricInfo` / `GetPlayLyricInfo` | `crypt:1, qrc:1, trans:1, songMid` | `lyric`/`trans` 都变成本文 §五 的密文 |
| 取流地址 | `resolve_song_url` | `music.vkey.GetVkey` / `UrlGetVkey` | `songmid`, `filename`, `guid` | 按音质阶梯探测；**必须 android 档案** |
| 收藏 / 取消收藏 | `set_liked` | `music.musicasset.PlaylistDetailWrite` / `AddSonglist`·`DelSonglist` | `songMid`（组件解析数字 id）或 `songId`，`liked` | 唯一的写操作 |
| 搜索（四类） | `search_songs` / `search_artists` / `search_albums` / `search_playlists` | `music.search.SearchCgiService` / `DoSearchForQQMusicMobile` | `keyword, num_per_page, page_num, search_type` 0/1/2/3 | 结果在 `body.item_*`，总数 `meta.sum`；标题带 `<em>`，组件剥掉 |
| 封面匹配（三件） | `search_track_artwork` / `search_artist_artwork` / `search_album_artwork` | 内部走搜索 | 名称字段 + `limit` | 候选 + 排名置信度 |

## 三、逐字歌词（QRC）

QQ 音乐的逐字数据只在**加密路**上：`GetPlayLyricInfo` 带 `crypt:1, qrc:1` 时，
`lyric` 字段不再是 base64 的 LRC，而是 **hex 编码的密文**；`trans`（翻译）同样是密文。
明文 fcgi 路只有整行歌词，拿不到逐字——这就是过去没有逐字时间的原因。

组件内部三步，互相独立：

1. **解密**（`src/qrc.rs`）——QQ 自己的「类 DES」（S/P/E 盒与密钥位序都是它私有的，不是标准 DES），
   三重组合 `D(K3) → E(K2) → D(K1)`，解密后是带 UTF-8 BOM 的 zlib，解压得到 QRC 文档。
   规格移植自 MIT 的 [qrc-decoder](https://github.com/apoint123/qrc-decoder)。
2. **取内容**——解出来的是一层 XML 壳，真正的歌词在一个属性（或 CDATA）里，实体转义过。
3. **转格式**——QRC 文本是 `[行开始,行时长]词(词开始,词时长)…`；
   组件把它转成 **LRC，但每个词前都带一个时间戳**：

   ```
   [00:00.00]五[00:00.43]百[00:01.13]英[00:01.65]里
   ```

   选这个形状是因为宿主的歌词读取器**一行的多个时间戳就当作多个词**，并据此生成逐字 TTML——
   宿主不必为逐字做任何特殊处理。

`fetch_lyric` 因此回答两个字段：`lyric`（整行，明文路，兜底用）与 `wordLyric`（逐字，有则给）。
**没有逐字不是错误**：老歌往往就没有，那时 `wordLyric` 为 `null`，宿主用 `lyric` 就好。
翻译优先取加密路解出来的那份（明文路的 `trans` 常常是空的）。

## 四、下载引擎（Aria2 Next）

组件在 `<自身目录>/aria2-next` 找到引擎后按需拉起，用标准 JSON-RPC 驱动
（回环端口、每次启动重新生成 `--rpc-secret`）。引擎缺席时这些方法报错，宿主应退回自己的下载方式。

| 方法 | 作用 |
|---|---|
| `aria2_status` | 是否安装/运行、版本、**实际在用**的端口、当前速度、生效参数。`ensure=true` 会顺手拉起 |
| `aria2_restart` | 停掉再拉起（换端口、清掉卡死任务用） |
| `aria2_configure` | 分块数、单服务器连接数、并发任务数、最小分块、总限速、**端口**；除端口外立即生效 |
| `aria2_add` | 排队一个文件：`url` + `out`（引擎目录内的文件名） |
| `aria2_tell` | 单个任务的进度，按 `gid` |
| `aria2_list` | 全部任务：`tellActive` + `tellWaiting` + `tellStopped` 合并 |
| `aria2_pause` / `aria2_unpause` / `aria2_cancel` | 单个（给 `gid`）或全部（不给）。取消**同时删掉临时文件** |

## 五、当前限制

- **微信扫码登录**未实现：QQ 扫码与网页 cookie 两条路径可用。
- **歌手简介多数为空**：上游本身对很多歌手没有这篇文字，空即答案。
