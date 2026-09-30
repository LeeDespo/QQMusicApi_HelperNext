# 解析与坑

上游响应的形状不统一，这一份记录**实测**出来的结论，避免后来的人重复踩。

## 1. 平台档案决定成败

`comm` 用 **web** 档案（`cv 4747474 / ct 24 / platform "yqq.json" / needNewCode 1`）。
用 QQMusicApi 库的默认档案请求"我喜欢"会得到 `10004`；同一份凭据、同一个接口，换 web 档案就正常。
**账号列表（我喜欢 / 我的歌单 / 收藏专辑 / 关注歌手）必须走 web 档案。**

## 2. 我喜欢的曲目在 `songlist`，总数在 `dirinfo.songnum`

`CgiGetDiss`（`dirid=201`，无 `disstid`）返回 `dirinfo`（`title`、`songnum`）与 `songlist`。
歌单/排行榜用同一个方法、给 `disstid`，总数同样来自 `dirinfo.songnum`——
这是"歌单只能显示前 100 首"的解药：知道总数才能判断"取完了没有"。

## 3. 账号自己的歌单与收藏专辑只有老 fcgi 能取

`music.musicasset.PlaylistBaseRead` 上的候选方法一律返回 `40000`。
必须走 `GET c.y.qq.com/fav/fcgi-bin/fcg_get_profile_order_asset.fcg`，
`userid` 用**数字 uin**，`reqtype` 3=歌单、2=专辑。
返回的 `cdlist` 里混着保留目录：**没有 `dissid` 的那些要跳过**（我喜欢就是这样一个目录）。

## 4. `pubtime` 是北京时间零点

收藏专辑的 `pubtime` 是 Unix 时间戳，且**恰好是 16:00Z**（抽查八张全部如此，例如
流浪地球 → `2019-02-04T16:00Z`）。按 UTC 渲染会让**每一张**专辑都早一天。
本组件按 +08:00 换算（`album_release_date`），得到的就是服务里显示的日期。

## 5. 大小写与命名

* 关注歌手的列表键是 **`List`**（大写 L）。QQMusicApi 的模型里叫 `users`，照模型去找会以为"没有这个能力"。
* `HostUin` 要的是**加密后的 uin**（`encrypt_uin`），不是数字 uin，库也不自己推导——凭据文件里有。
* 曲目里歌手的**分隔符统一为 `", "`**；多个歌手就是多个 `singer` 元素，逐个映射成 `singers`。
* 字段名在不同接口里不同（`mid`/`songmid`/`albummid`、`interval`/`duration`），
  取值一律用 `upstream.rs` 的候选键工具，不要写死单键。

## 6. 曲目字段（`decode_track` 的映射）

| 输出 | 来源（按优先级） |
|---|---|
| `songMid` | `mid` / `songMid` / `songmid`（缺失→该条目丢弃） |
| `songId` | `id` / `songId` / `songid` |
| `title` | `name` / `title` / `songname` |
| `artist` | `singer[].name` 用 `", "` 连接 |
| `album` / `albumMid` | `album.name` / `album.mid` |
| `albumId` | `album.id` / `album.albumId`（**数字**，专辑页要用它） |
| `imageURL` | 由 `albumMid` 拼 `T002R800x800M000<mid>.jpg` |
| `duration` | `interval` / `duration`（秒） |
| `payPlay` | `pay.pay_play`（嵌套，不是扁平键） |

## 7. 封面 URL 一律转 https

上游会给 `http://y.gtimg.cn/...`、`http://qpic.y.qq.com/...` 以及协议相对的 `//qpic.y.qq.com/...`。
宿主（macOS 应用没有 ATS 例外，Android 9+ 默认禁止明文）会直接拒绝 http 图片，
表现为"封面静默空白"。所有封面都过 `normalized_artwork_url`。

## 8. 每个响应都要回带请求的 `id`

子进程适配器按 `id` 把响应配给请求。少写一个 `id` 不会报错，只会让对方**等到超时**
（macOS 应用是 15 秒）。这一条在 Python 版上真的发生过：新方法输出 `{"ok":true,…}`，
helper 单测全绿，接进应用就"卡 15 秒"。`with_id` 现在集中处理。

## 9. 登录：`qm_keyst` 就是全部

扫码登录与网页登录产出的都是 `uin` + `qm_keyst` 这一对；
`g_tk = hash33(qm_keyst)` 由此推导，`qm_keyst` 同时就是 VIP 取流所需的播放票据。
所以 `import_credential(uin:qm_keyst:)` 不需要别的 cookie。

`GetLoginUserInfo` 的响应里，昵称在 `info.nick`（不是 `nickname`）；凭据被拒时要报成"未登录"，
而不是抛错——那不是失败，是一种答案。

## 10. 限流与熔断的位置

组件是唯一看得见全部流量的地方，所以两个机制放在这里：
按内容类别分桶的固定窗口（超限**等待**，因为每次调用都是用户看得见的读取），
以及熔断（60 秒内 5 次失败开路 30 秒，半开只放一个探测）。
`get_status` 把两者的状态暴露出来，便于回答"是我在限流自己，还是上游在拒绝我"。


## 11. 设备标识：已实现（qimei），但只解决了一半（2026-09-30 实测）

`src/device.rs` 现在自己完成整套 QIMEI 握手：生成并持久化一个设备（IMEI/OpenUDID/android_id/
机型/系统版本），用 RSA 包一把随机 AES 密钥、AES-CBC 加密设备档案、两处 MD5 签名，POST 到
`api.tencentmusic.com/tme/trpc/proxy`，拿到 `q16`/`q36` 缓存 24 小时。**实测通**（拿到真实 q16/q36），
并且密码学部分有单元测试钉住：MD5 已知摘要、AES-128-CBC 的 NIST SP 800-38A 向量。

一个坑：库里的公钥是 PEM，但 `rsa` crate 的 PEM 读取路径拒收它（`openssl` 与 base64 都接受），
所以改成直接内嵌 **DER 字节**（同一把密钥），绕开 PEM 解析。

**结果分两半：**

| 接口 | 有了设备标识之后 |
|---|---|
| `fetch_artist_detail`（歌手资料） | ✅ **修好**（android 档案 + `qq`/`authst` + 设备字段；实测 周杰伦） |
| `search_*`（搜索四类） | ❌ 仍然 `meta.sum = 0`（**用真实 qimei 直连也一样**，所以不是 qimei 的事） |
| `fetch_recommend_feed`（猜你喜欢） | ❌ 仍然 0 |
| `set_liked`（收藏，写） | ❌ 仍然 `code 1000` |

### 会话（`uid`/`sid`）：已实现，且**修好了猜你喜欢**

```
POST musicu.fcg  module = music.getSession.session, method = GetSession
param = {"uid": "", "vkey": 0, "caller": 0}     （comm 必须是 android 那一套 + QIMEI）
→ data.session.{uid, sid, vkey}                 组件缓存 24 小时，随后的 android 调用带上 uid/sid
```
实测：`uid=6791462615`、`sid=20260930193553` 拿到并持久化，**猜你喜欢立刻从 0 变 5 首**
（Only you can save me / かいじゅうのマーチ / I Remember）。所以这些接口要的是
"**登录过的设备会话**"，而不只是"设备指纹"——两个都要。

**仍然失败的只剩收藏（写）**：`code 1000`。设备指纹与会话都齐了之后还是 1000，
说明写操作还要别的东西（可能是与登录方式匹配的 `tmeLoginType`，或者用真正登录态建立的凭据）。
下一步排查方向：与 Python 版**逐字段**比对同一次写请求的 comm（两边都 dump 原始请求体）。

## 12. 历史记录：三个接口同因的排查过程（已由第 11 条取代）

下面三个接口都不是参数写错，而是**上游要求一个设备身份**（`qimei` / `qimei36` 等字段），
QQMusicApi 的 `utils/device.py` 就是专门去取它并缓存、再塞进 `comm` 的：

已查明设备标识不是"补一个字段"：`utils/qimei.py` 向 `api.tencentmusic.com/tme/trpc/proxy`
POST 一个 **RSA 加密的 AES 密钥 + AES 加密的 payload**，依赖整套设备字段。所以这是一次独立的移植。

| 接口 | 现象 |
|---|---|
| `music.search.SearchCgiService/DoSearchForQQMusicMobile` | `code 0`，`meta.sum = 0`（两个档案都一样） |
| `music.radioProxy.MbTrackRadioSvr/get_radio_track`（猜你喜欢） | `code 1000`，`tracks: []`（两个档案都一样） |
| `music.UnifiedHomepage.UnifiedHomepageSrv/GetHomepageHeader`（歌手资料） | `code 10000`，`Info.Singer` 每个字段都是空串，只有 `Info.FansNum/FollowNum/IP` 有值 |
| `music.musicasset.PlaylistDetailWrite/AddSonglist`（收藏，唯一的写） | `code 1000`（五种参数形状都一样） |

而**不需要**设备身份的接口（我喜欢、歌单、专辑、歌手歌曲/专辑、榜单、电台列表、新歌、取流）都正常。
所以下一步应当**先实现设备标识**，它大概率一次性解锁这四条；在此之前
`fetch_artist_detail` 明确报错而不是回一个"未知歌手"的假资料，`set_liked` 返回上游错误码。

歌词是**例外**且已解决：它不需要设备标识，换成明文 fcgi 路即可（见 `docs/endpoints.md`）。
