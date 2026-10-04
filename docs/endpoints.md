# 接口清单

组件对外暴露的每一个方法，以及它背后打的上游接口。**这一份是参考手册，不是开发记录**：
状态列写的是当前实现，不记日期、不记排查过程；那些坑记在[解析要点](parsing.md)里。

## 一、上游与请求信封

| 上游 | 形状 | 用途 |
|---|---|---|
| `POST https://u.y.qq.com/cgi-bin/musicu.fcg` | `{comm:{…}, req_0:{module, method, param}}` | 目录、账号列表、取流、登录 |
| `POST https://u.y.qq.com/cgi-bin/musics.fcg?sign=…` | 同一份 `{comm, req_0}` 信封 + URL 上的 `zzc` 签名 | 乐谱、不喜欢家族、歌手名称图像（见[解析要点](parsing.md) §15） |
| `GET https://u.y.qq.com/cgi-bin/musicu.fcg?format=json&data=…` | 整个信封编码进 `data` | 登录扩展（要按业务码分派、要每次请求的 comm 覆盖，现有信封 API 覆盖不了） |
| `GET https://c.y.qq.com/fav/fcgi-bin/fcg_get_profile_order_asset.fcg` | 表单式 query，`userid` 用数字 uin，`reqtype` 2=专辑 3=歌单 | 账号自己的歌单与收藏专辑（**只有它**能取） |
| `GET https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg` | 表单式 query，`nobase64=1` | 明文歌词 |
| `GET https://c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg` | 表单式 query，`key` 是唯一参数 | 快速搜索 |
| `ssl.ptlogin2.qq.com` · `graph.qq.com` | 扫码登录的五步握手 | QQ 扫码登录 |
| `open.weixin.qq.com` | 页面 HTML、二维码图片、状态长轮询 | 微信扫码登录 |

**`comm` 的平台档案按接口分类选择**，选错不报错、只回空数据：

- **web 档案** —— 账号列表（我喜欢 / 我的歌单 / 收藏专辑 / 关注歌手）、曲库详情、榜单、电台、新歌；
  移植层里的评论、MV、账号资产与关系、集合写入、推荐扩展也是这一档（这些端点参考没标档案，跟随调用方给的档案，组件默认 Web）。
  用库默认档案请求账号列表会被拒（`10004`）。
- **android 档案** —— 搜索、取流、既有 `set_liked`（收藏写）、推荐流、歌手资料与主页 Tab、封面匹配、
  批量取流；两个搜索 CGI 端点（综合 / 类型搜索）按实测也走这里。
  这些接口要**设备身份**与**设备会话**，两者都在 android 那套 `comm` 里。

## 二、方法清单

除非另有说明，每条都在真实账号上跑通过。移植层里只做过单测、没上真机的条目会在下文另注；
写接口与需要扫码/手机号的接口只在单测层验证（见第五节的当前限制）。

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
| 歌手简介 | `fetch_artist_biography` | `music.musichallSinger.SingerInfoInter` / `GetSingerDetail` | `singer_mids` 与数字 `1` 开关；主页资料另读 Header | 回答在 `artistDetail`，优先 `ex_info.desc` |
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
| 收藏 / 取消收藏 | `set_liked` | `music.musicasset.PlaylistDetailWrite` / `AddSonglist`·`DelSonglist` | `songMid`（组件解析数字 id）或 `songId`，`liked` | 移植前唯一的写操作；移植层另有集合写入一族 |
| 搜索（四类） | `search_songs` / `search_artists` / `search_albums` / `search_playlists` | `music.search.SearchCgiService` / `DoSearchForQQMusicMobile` | `keyword, num_per_page, page_num, search_type` 0/1/2/3 | 结果在 `body.item_*`，总数 `meta.sum`；标题带 `<em>`，组件剥掉 |
| 封面匹配（三件） | `search_track_artwork` / `search_artist_artwork` / `search_album_artwork` | 内部走搜索 | 名称字段 + `limit` | 候选 + 排名置信度 |

移植层（`src/port/`）的接口按领域另列一张表。参考库里没有标档案的端点按本层约定跟随调用方给的档案（组件默认 Web），
表里另注的除外。

| 能力 | 方法 | 上游 module / method | 参数要点 | 关键回值 |
|---|---|---|---|---|
| 评论 · 数量 | `fetch_comment_count` | `music.globalComment.CommentCountSrv` / `GetCmCount` | `bizId`（数字），`bizType` 1=歌曲 2=专辑 3=歌单 4=MV 15=长音频（缺省 1），`bizSubType`（歌曲缺省 2） | 总数在 `count`，角标在 `iconList`，`cmTabType` 在 `response` 的兄弟键上 |
| 评论 · 热评 | `fetch_hot_comments` | `music.globalComment.CommentRead` / `GetHotCommentList` | `bizId, page, limit, lastCommentSeqNo, bizType` | `comments` + `hasMore`；`PageNum` 是 `page-1`（照参考） |
| 评论 · 新评 | `fetch_new_comments` | 同上 / `GetNewCommentList` | 同上 | 同上 |
| 评论 · 推荐评 | `fetch_recommend_comments` | 同上 / `GetRecCommentList` | 同上 | 同上 |
| 评论 · 时刻评论 | `fetch_moment_comments` | `music.globalComment.SongTsComment` / `GetSongTsCmList` | `bizId, limit, lastCommentSeqNo`（游标回填回值 `nextPos`） | `comments` + `nextPos` |
| 评论 · 发表 | `add_comment` | `music.globalComment.CommentWriteServer` / `AddComment` | `bizId, content, replyCmtId, bizType`；需登录 | 新评论 `id`、`subcode`，必要时给验证码 URL |
| 评论 · 删除 | `delete_comment` | 同上 / `DelComment` | `cmId`；需登录 | `SubCode==0` 即成功，**评论不存在也算成功** |
| MV · 详情 | `fetch_mv_detail` | `video.VideoDataServer` / `get_video_info_batch` | `vids`（协议键 `vidlist`）+ 22 项 `required` 长数组（照参考逐字抄，含重复项） | 以 vid 为键的映射；空映射是答案 |
| MV · 播放地址 | `resolve_mv_urls` | `music.stream.MvUrlProxy` / `GetMvUrls` | `vids`；`guid` 每次请求现生成 | 以 vid 为键，每条含 `mp4`/`hls` 两组 |
| MV · 分类列表 | `fetch_mv_list` | `MvService.MvInfoProServer` / `GetAllocMvInfo` | `area`(15=全部), `version`(7=全部), `order`(0=最新), `num`, `page`；`start = num*(page-1)` | `total` + `items`（上游键是 `list`） |
| 歌手 · 列表 | `fetch_singer_list` | `music.musichallSinger.SingerList` / `GetSingerList` | `area`/`sex`/`genre` 照 AreaType/SexType/GenreType，缺省都是 -100 | `singerlist` / `hotlist` / `tags`；空列表是答案 |
| 歌手 · 索引分页 | `fetch_singer_index` | 同上 / `GetSingerListIndex` | 同上 + `index`（IndexType，缺省 -100）、`page`、`num` | 比列表多 `index` / `total`；请求里 `sin` 是偏移、`cur_page` 是页码 |
| 歌手 · 相似 | `fetch_similar_artists` | `music.SimilarSingerSvr` / `GetSimilarSingerList` | `singerMid`, `number`（缺省 10） | `singerlist` |
| 歌手 · 主页 Tab | `fetch_artist_tab` | `music.UnifiedHomepage.UnifiedHomepageSrv` / `GetHomepageTabDetail` | `singerMid`, `tabType`（字符串 TabType，或按声明次序 1..=9）, `page`, `num`；**调用方没给档案时缺省 android** | `TabID`/`HasMore` 与各 Tab 的内容；web 档案只回空壳（`10000`） |
| 歌手 · 名称图像 | `fetch_artist_display_name` | 同上 / `GetHomepageHeader` | `singerMid`；走签名路、comm 盖 `cv/v = 20080000` | `displayType` / `picFile` / `name`；无名称图片时 displayType 0 |
| 歌手 · MV | `fetch_artist_mvs` | `MvService.MvInfoProServer` / `GetSingerMvList` | `singerMid, num, page`；`start = (page-1)*num` | `total` + `mvList` |
| 歌曲资产 · 批量信息 | `query_songs` | `music.trackInfo.UniformRuleCtrl` / `CgiGetTrackInfo` | `songs[]`，每项 `id` 或 `mid` **二选一**，可带 `songType` | 曲目列表；空是答案 |
| 歌曲资产 · CDN 调度 | `fetch_cdn_dispatch` | `music.audioCdnDispatch.cdnDispatch` / `GetCdnDispatch` | `{}` | `sip` 里的根地址用来拼取流回值的 `purl` |
| 歌曲资产 · 批量取流 | `resolve_song_urls` | `music.vkey.GetVkey` / `UrlGetVkey`（顶层 `fileType` 属加密类时改 `music.vkey.GetEVkey` / `CgiGetEVkey`） | `fileInfo[]`（`mid`/`fileType`/`songType`/`mediaMid`），`fileType` 缺省 `MP3_128`；一次 ≤100 mid；缺省 android | 每项一个结果码；`purl` 是相对路径。加密档可能回 `ekey: null` |
| 歌曲资产 · 其他版本 | `fetch_other_versions` | `music.musichallSong.OtherVersionServer` / `GetOtherVersionSongs` | `value`：数字串当歌曲 ID，否则当 MID | `versionList`；空是答案 |
| 歌曲资产 · 制作人 | `fetch_song_producer` | `music.sociality.KolWorksTag` / `SongProducer` | 同上 | 制作人列表；空是答案 |
| 歌曲资产 · 收藏数 | `fetch_song_fav_count` | `music.musicasset.SongFavRead` / `GetSongFansNumberById` | `songIds[]` | `numbers` 以歌曲 ID 为键 |
| 歌曲关联 · 相似歌曲 | `fetch_similar_songs` | `music.recommend.TrackRelationServer` / `GetSimilarSongs` | `songId`（协议键 `songid`） | 按组给标题 + 曲目 |
| 歌曲关联 · 标签 | `fetch_song_labels` | 同上 / `GetSongLabels` | 同上 | 标签列表 |
| 歌曲关联 · 相关歌单 | `fetch_related_playlists` | 同上 / `GetRelatedPlaylist` | `songId`, `last`（上一批歌单 ID，换一批用） | 歌单列表 |
| 歌曲关联 · 相关 MV | `fetch_related_mvs` | `MvService.MvInfoProServer` / `GetSongRelatedMv` | `songId`, `lastMvid`（缺省数字 0） | MV 列表 |
| 歌曲关联 · 曲谱 | `fetch_sheet_music` | `music.mir.SheetMusicSvr` / `GetMoreSheetMusic`（`ttype=2` 时 `GetChongChongSheetMusic`） | `mid`, `ttype`(0=用户 1=AI 2=虫虫)；**签名路** + 匿名 h5 comm | 曲谱数据；`10007`（没有曲谱）还原成空答案 |
| 歌曲关联 · 是否有曲谱 | `has_sheet_music` | 同上 / `HasSheetMusic` | `mid`；**签名路** + 匿名 h5 comm | 布尔开关（`hasLdy`/`hasQrcx`/…） |
| 账号资产 · 收藏歌单 | `fetch_fav_playlists` | `music.musicasset.PlaylistFavRead` / `CgiGetPlaylistFavInfo` | `euin`（缺省凭据）, `page`, `num` | `v_list` 缺席即形状故障，键在而空是答案 |
| 账号资产 · 收藏专辑 | `fetch_fav_albums` | `music.musicasset.AlbumFavRead` / `CgiGetAlbumFavInfo` | 同上 | 同上 |
| 账号资产 · 收藏 MV | `fetch_fav_mvs` | `music.musicasset.MVFavRead` / `getMyFavMV_v2` | 同上；需登录 | `mvlist` 缺席即故障 |
| 账号资产 · 音乐基因 | `fetch_music_gene` | `music.recommend.UserProfileSettingSvr` / `GetProfileReport` | `euin`（缺省凭据） | `UserInfoCard` 是形状锚点；`ListeningReport` 可缺 |
| 账号资产 · 不喜欢列表 | `fetch_dislike_list` | `music.feedback.FeedbackBlack` / `GetDislikeList` | `cmd` 2=歌手 3=歌曲 4=风格, `page`, `lastid`；**签名路**；需登录 | `singers`/`songs`/`styles`；空是答案 |
| 账号资产 · 加/取消不喜欢 | `add_dislike` / `cancel_dislike` | 同上 / `AddDislike`·`CancelDislike` | `idType` 1=歌曲 2=歌手 3=风格, `values[]`；**签名路**；需登录 | `Retcode == 0` |
| 账号资产 · 清空不喜欢歌曲 | `clear_dislike_songs` | 同上 / `CancelAllDislike` | 两步：先 `ISOnlyGetToken` 取 Token，再 `DelType=3`；**签名路**；需登录 | 第二步 `Retcode == 0` |
| 账号关系 · 主页 | `fetch_user_homepage` | `music.UnifiedHomepage.UnifiedHomepageSrv` / `GetHomepageHeader` | `euin`（缺省凭据）；未登录时用占位凭证 | `Info.BaseInfo` 是形状锚点 |
| 账号关系 · VIP | `fetch_vip_info` | `VipLogin.VipLoginInter` / `vip_login_base` | `{}`；需登录 | `identity` / `userinfo` |
| 账号关系 · 关注的歌手 | `fetch_follow_singers` | `music.concern.RelationList` / `GetFollowSingerList` | `euin, page, num`（协议键 `HostUin`/`From`/`Size`）；需登录 | `List` 是形状锚点；`Total`/`HasMore` 原样回 |
| 账号关系 · 粉丝 | `fetch_fans` | 同上 / `GetFansList` | 同上 | 同上 |
| 账号关系 · 好友 | `fetch_friends` | `music.homepage.Friendship` / `GetFriendList` | `page, num`（`Page = page-1`）；需登录 | `friends` + `hasMore` |
| 账号关系 · 关注的人 | `fetch_followed_users` | `music.concern.RelationList` / `GetFollowUserList` | 同关注的歌手 | 同上 |
| 账号关系 · 创建的歌单 | `fetch_created_playlists` | `music.musicasset.PlaylistBaseRead` / `GetPlaylistByUin` | `uin`（数字；缺省凭据的 music id） | `v_playlist` 是形状锚点；`bFinish` 标记是否完整 |
| 推荐扩展 · 首页信息流 | `fetch_home_feed` | `music.recommend.RecommendFeed` / `get_recommend_feed` | `page, direction, s_num, v_cache` | 楼层卡片；续参由回值拼（direction=1、page+1、s_num+v_cache） |
| 推荐扩展 · 雷达推荐 | `fetch_radar_recommend` | `music.recommend.TrackRelationServer` / `GetRadarSong` | `page` | 歌曲列表 + `hasMore` |
| 推荐扩展 · 推荐歌单 | `fetch_recommend_playlists` | `music.playlist.PlaylistSquare` / `GetRecommendFeed` | `page, num`（缺省 25） | `songlists` + `fromLimit`（下一页的 `From` 用它） |
| 搜索扩展 · 热搜词 | `fetch_search_hotkeys` | `music.musicsearch.HotkeyService` / `GetHotkeyForQQMusicMobile` | `{}` | `vecHotkey` |
| 搜索扩展 · 联想补全 | `complete_search` | `music.smartboxCgi.SmartBoxCgi` / `GetSmartBoxResult` | `keyword` | `items` 补全建议 |
| 搜索扩展 · 快速搜索 | `quick_search` | `GET c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg` | `keyword`（query 的 `key`） | song/singer/album/mv 四个分类 |
| 搜索扩展 · 综合搜索 | `general_search` | `music.adaptor.SearchAdaptor` / `do_search_v2` | `keyword, page, num, searchid, pageStart, highlight`；**缺省 android**（web 档案回 2001 风控） | 分桶结果 + `meta`；续参 `searchid`/`nextpage`/`nextpageStart` |
| 搜索扩展 · 类型搜索 | `search_extra` | `music.search.SearchCgiService` / `DoSearchForQQMusicMobile` | `keyword, searchType`（0/1/2/3/4/7/8/10/15/18）, `page, num, selectors, highlight`；**缺省 android** | `body.item_*`，总数 `meta.sum`，`selectors` 是「组数组的数组」 |
| 集合写入 · 建歌单 | `create_playlist` | `music.musicasset.PlaylistBaseWrite` / `AddPlaylist` | `dirName`；需登录 | `dirid`；重名自动加时间戳，不会失败 |
| 集合写入 · 删歌单 | `delete_playlist` | 同上 / `DelPlaylist` | `dirId`；需登录 | 删除不存在的歌单时回值 `dirid` 是 0 |
| 集合写入 · 歌单加歌 | `add_playlist_songs` | `music.musicasset.PlaylistDetailWrite` / `AddSonglist` | `dirid, songInfo[]`（`{songId, songType}`）, `tid`；需登录 | `80092`（已在歌单里）按成功处理 |
| 集合写入 · 歌单删歌 | `remove_playlist_songs` | 同上 / `DelSonglist` | 同上；需登录 | `80092`（不在歌单里）按成功处理 |
| 集合写入 · 收藏专辑 | `fav_album` | `music.musicasset.AlbumFavWrite` / `FavAlbum` | `albumIds[]`；需登录 | 逐张结果 |
| 集合写入 · 取消收藏专辑 | `unfav_album` | 同上 / `CancelFavAlbum` | 同上；需登录 | 同上 |
| 登录扩展 · 微信取码 | `fetch_wx_qrcode` | `open.weixin.qq.com` 页面 + 二维码图片 | `{}` | `qrcode`（`loginType` 是 `"wx"`） |
| 登录扩展 · 微信查状态 | `check_wx_qrcode` | 状态长轮询 + `music.login.LoginServer` / `Login` | `identifier`；`DONE` 时 `tmeLoginType=1` 换凭据 | `SCAN`/`CONF`/`REFUSE`/`TIMEOUT`/`DONE`；凭据落盘 |
| 登录扩展 · 发手机验证码 | `send_phone_authcode` | `music.login.LoginServer` / `SendPhoneAuthCode` | `phone`（数字）或 `encryptedPhone`（二选一）, `countryCode`；固定 android | `20276` 滑块验证、`100001` 发送频繁都是答案 |
| 登录扩展 · 验证码登录 | `phone_login` | 同上 / `Login` | 号码 + `authCode`；固定 android | 成功后凭据落盘 |
| 登录扩展 · 刷新凭据 | `refresh_credential` | 同上 / `Login` | `{}`；按凭据 `loginType` 分三套参数 | 新凭据写回存储 |

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

## 五、补充接口

| 方法 | 上游 module / method | 参数与结果 |
|---|---|---|
| `fetch_new_albums` | `newalbum.NewAlbumServer / get_new_album_info` | `area=1, num=20, page=1`；返回 `total, albums`；协议也接受 `limit` |
| `fetch_user_liked_songs` | `music.srfDissInfo.DissInfo / CgiGetDiss` | 必填目标 `euin`，`page, num`；返回 `info, songs, size, total, hasmore` |
| `fetch_singing_annotations` | `music.musichallSong.PlayLyricInfo / GetSingingAnnotationsInfo` | `songId`；返回 `hasSingingAnnotationsLyric` |
| `fetch_multi_style_lyrics` | 同模块 / `BatchGetMultiStyleTransLyric` | `songId`；返回 `lyrics`，自动尝试 QRC 解密 |
| `has_ai_dictionary` | 同模块 / `IsAIDictExists` | `songId`；返回 `exists` |
| `fetch_ai_dictionary` | 同模块 / `GetAIDictInfo` | `songId`；返回 `dictList` |
| `fav_playlist` / `unfav_playlist` | `music.musicasset.PlaylistFavWrite / FavPlaylist`·`CancelFavPlaylist` | 公开歌单 `playlistId`（disstid，不是 dirid），需要登录；返回 `success` |

### 子进程 ID 约定

请求/回复顶层的 `id` 始终是请求关联标识。创建歌单、发表评论等响应的资源 `id` 在子进程协议中改放 `resultId`；例如创建歌单回复的 `resultId` 是 disstid，`dirid` 是目录 ID。FFI 直接调用的资源字段仍叫 `id`。

### 已验证与剩余限制

- 2026-10-04 完成真实账号只读、分页、搜索类型、类型化模型及可逆写入验证；详见[接续报告](continuation-verification-2026-10-04.md)。
- 私信与 COS 上传仍是原工作流延期模块；手机 App MQTT 扫码也未移植。
- 本轮按用户要求跳过所有登录/扫码测试，不查看截图。
- 清空不喜欢列表仅验证获取 Token 的预检，不执行全量删除，以保留既有条目的时间和顺序。缺失 Token 或非零业务码会阻止删除。
- `fetch_other_versions` 的真实样本可能为空；无附加翻译/词典的 `null` 归一为空列表，错误类型仍报错。
- `songTsElems`/`hashTagList`/`iconList`/`subComments` 等任意 JSON 字段在 FFI 中保留为 JSON 文本；子进程仍返回对象/数组。
- 加密取流的 `ekey`/`purl` 无内容时保留上游结果；宿主应根据授权结果选择普通音质。
