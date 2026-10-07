# 解析要点

上游响应的形状不统一，这一份记录那些**不直观、但必须照做**的地方。
每一条都是结论；推导过程在提交信息里，不在这里。

## 1. 平台档案按接口选，选错只回空数据

`comm` 有两套档案，用途不同（见[接口清单](endpoints.md)第一节）：

- **web**（`cv 4747474 / ct 24 / platform "yqq.json" / needNewCode 1`）——账号列表、曲库详情、榜单、电台、新歌。
- **android**（带设备身份与设备会话）——搜索、取流、收藏写、推荐流、歌手资料、封面匹配。

用错档案不会报错：搜索会回 `meta.sum = 0`，取流会回 `104003`。所以"某个接口今天突然没数据了"
先看档案，再看参数。

扩展端点层按端点的实测结论选档案，两处容易记反：

- **必须 android，且是实测确定的**：歌手主页 Tab（`GetHomepageTabDetail`，web 档案回 `code 10000` + 空壳）、
  歌手名称图像（`GetHomepageHeader` 且 comm 要盖 `cv/v = 20080000`）、
  综合搜索 `do_search_v2`（web 档案直接回 `code 2001` 风控）、类型搜索 `DoSearchForQQMusicMobile`
  （web 档案回空目录 `meta.sum = 0`）。前两者调用方显式给 `platform` 时尊重调用方；后两者缺省 Android
  与既有 `search_songs` 一致。
- **反过来，`GetSingerList` 在 android 档案下被拒**（`104403`），所以歌手列表跟随调用方档案
  （组件默认 Web），不要按"歌手资料走 android"一刀切。

## 2. 我喜欢的曲目在 `songlist`，总数在 `dirinfo.songnum`

`CgiGetDiss`（`dirid=201`，不给 `disstid`）返回 `dirinfo`（含 `title`/`songnum`）与 `songlist`。
歌单与排行榜用同一个方法、给 `disstid`，总数同样来自 `dirinfo.songnum`——
知道总数才能判断"取完了没有"，这是"列表只显示前 100 首"的解药。

## 3. 账号自己的歌单与收藏专辑只有老 fcgi 能取

`music.musicasset.PlaylistBaseRead` 上的候选方法一律返回 `40000`。
必须走 `GET c.y.qq.com/fav/fcgi-bin/fcg_get_profile_order_asset.fcg`，
`userid` 用**数字 uin**，`reqtype` 3=歌单、2=专辑，并且要带 `format=json`。
返回的 `cdlist` 里混着保留目录：**没有 `dissid` 的那些要跳过**（「我喜欢」就是这样一个目录）。
`data` 为空或 `code != 0` 要**报错而不是回空列表**——"这个账号没有歌单"是合法答案，
会被宿主缓存下来，用它覆盖缓存里的真数据就是丢用户的东西。

## 4. `pubtime` 是北京时间零点

收藏专辑的 `pubtime` 是 Unix 时间戳，且**恰好是 16:00Z**（抽查八张全部如此，
例如流浪地球 → `2019-02-04T16:00Z`）。按 UTC 渲染会让每一张专辑都早一天。
按 +08:00 换算（`album_release_date`）才是服务里显示的日期。

## 5. 大小写与命名

- 关注歌手的列表键是 **`List`**（大写 L）。不要按语义猜成 `users`；否则会把正常响应误判为空。
- 专辑列表里是 **`albumID`**（大写 ID）。按 `albumId` 取会每一项都拿不到 id，
  整表被 `filter_map` 静默丢掉、返回 0 张。
- 专辑曲目的列表键是 **`songList`**（大写 L），不是 `songlist`。
- 排行榜的分组键是 **`toplist`**（小写 l），而曲目在 **`songInfoList`**——
  `data.data.song` 是展示用行（只有 rank/title/封面，**没有 song mid**），拿它解码会得到空列表。
- `HostUin` 收**数字 uin 也可以**（与加密 uin 结果一致），不必依赖登录流程并不产出的 `encrypt_uin`。
- 曲目里多个歌手是**多个 `singer` 元素**，分隔符统一为 `", "`。
- 字段名在不同接口里不同（`mid`/`songmid`/`albummid`、`interval`/`duration`）：
  取值一律用 `upstream.rs` 的候选键工具，**不要写死单键**——候选键少一个拼写就是一张空表。

## 5b. 列表总数在不同接口的不同键上

能翻页的前提是知道总数，而它在每个接口的名字都不一样：

| 列表 | 总数键 |
|---|---|
| 我喜欢 / 歌单 | `dirinfo.songnum` |
| 排行榜 | `totalNum` |
| 专辑曲目 | `totalNum` |
| 关注歌手 / 搜索 | `meta.sum`（搜索）、接口自报（歌手） |

**没有总数的接口（专辑曲目的某些形状、电台）要按"取到空页为止"处理**，
不要拿"这一页的条数"当总数——那会让宿主以为已经取完了。

## 6. 曲目字段（`decode_track` 的映射）

| 输出 | 来源（按优先级） |
|---|---|
| `songMid` | `mid` / `songMid` / `songmid`（缺失→该条目丢弃） |
| `songId` | `id` / `songId` / `songid` |
| `title` | `name` / `title` / `songname` |
| `artist` | `singer[]`/`singers[]` 的 `name` 用 `", "` 连接；数组缺席或全无名时回退曲目顶层的 `singer`/`singername`/`singerName` 文本 |
| `singers` / `singerMid` | `singer`/`singers` 数组，元素取 `mid`/`singerMid`/`singerMID` 与 `name`；`singerMid` 缺省是第一位歌手的 mid，再看曲目顶层同名字段 |
| `album` / `albumMid` | `album.name` / `album.mid` |
| `albumId` | `album.id` / `album.albumId`（**数字**，专辑页要用它） |
| `imageURL` | 由 `albumMid` 拼 `T002R800x800M000<mid>.jpg` |
| `duration` | `interval` / `duration`（秒） |
| `payPlay` | `pay.pay_play`（嵌套，不是扁平键） |
| `mediaMid` | `file.media_mid` / `file.mediaMid`，再看曲目顶层同名字段 |
| `genre` | `genre` / `genreId` / `genre_id`（可选数字码，缺失保持 `null`） |
| `fileSizes` | `file.size_*` 映射为 `[{name,bytes}]`，如 `size_128mp3` → `{name:"128mp3",bytes:…}`；旧响应缺失时为空数组 |

`Album.songCount` 是可选字段；专辑搜索读取 `song_count`/`song_num`/`songNum`，歌手专辑列表优先用列表自带值，
缺失项再按每批最多 30 张专辑批量读取曲数。

## 6b. 分页使用实际行偏移

`fetch_playlist_tracks_page`、`fetch_album_tracks` 和 `fetch_artist_songs_page` 都接收 `offset`/`limit` 并保留上游 `total`。
曲目页额外回 `nextOffset`：它按上游原始列表行数递增，包含因缺少 mid 等原因无法解码为 `Track` 的行。
因此 `total` 可以大于所有页里可播放曲目的总数；下一页必须使用 `nextOffset`，不能用已解码 `tracks.count` 推算，
否则会重复请求过滤掉的行并卡在末尾。歌手专辑页仍按其返回专辑行分页。
关注歌手 `fetch_follow_singers` 可直接传 `offset`，否则仍按 `page`/`num` 兼容旧契约。总数缺失表示 `null`，
电台轮播 `fetch_radio_track_batch` 的 `total` 固定为 `null`；宿主用实际返回行数推进并自行去重。

## 7. 封面 URL 一律转 https

上游会给 `http://y.gtimg.cn/...`、`http://qpic.y.qq.com/...` 以及协议相对的 `//qpic.y.qq.com/...`。
宿主会直接拒绝 http 图片（macOS 应用没有 ATS 例外，Android 9+ 默认禁止明文），
表现为"封面静默空白"。所有封面都过 `normalized_artwork_url`。
歌单搜索结果的封面在 **`logo`** 键上，不在 `imgurl`/`picurl`。

## 8. 每个响应都要回带请求的 `id`

子进程适配器按 `id` 把响应配给请求。少写一个 `id` 不会报错，只会让对方**等到超时**
（macOS 应用是 15 秒）。`with_id` 集中处理这件事，新增方法时照抄相邻分支。

## 9. 登录：`qm_keyst` 就是全部

扫码登录与网页 cookie 导入产出的都是 `uin` + `qm_keyst` 这一对；
`g_tk = hash33(qm_keyst)` 由此推导，`qm_keyst` 同时就是 VIP 取流所需的播放票据。
所以 `import_credential(uin:qm_keyst:)` 不需要别的 cookie。

凭据被拒时要报成"未登录"，而不是抛错——那不是失败，是一种答案。

## 10. 设备身份与设备会话是两件事，都要

- **设备身份**（`src/device.rs`）：完整的 QIMEI 握手——生成并持久化一个设备（IMEI/OpenUDID/
  android_id/机型/系统版本），用 RSA 包一把随机 AES 密钥、AES-CBC 加密设备档案、两处 MD5 签名，
  POST 到 `api.tencentmusic.com/tme/trpc/proxy`，拿到 `q16`/`q36` 缓存 24 小时。
- **设备会话**（`music.getSession.session` / `GetSession`，`param = {"uid":"","vkey":0,"caller":0}`）：
  用 android 档案 + 设备身份换 `session.{uid, sid, vkey}`，随后的 android 调用带上它们。

搜索、推荐流、歌手资料要的是"**登录过的设备会话**"，而不只是设备指纹——两个都要。

一个实现上的坑：库里的公钥是 PEM，而 `rsa` crate 的 PEM 读取路径拒收它（`openssl` 与 base64 都接受），
所以直接内嵌 **DER 字节**（同一把密钥），绕开 PEM 解析。

## 11. 同一个 `hash33` 有两种种子

```
g_tk       = hash33(qm_keyst, 5381)     ← 显式传 5381
ptqrtoken  = hash33(qrsig)              ← 默认种子 0
```

种子不对不会得到"算错一点点"的结果，而是**上游直接回 HTTP 403**，看不出是参数问题。
组件因此把两者分开成 `hash33(key)`（种子 5381，给 `g_tk`）与 `hash33_seeded(key, seed)`。
**新增任何带签名的接口时，先去库里核对它用的种子**，别默认 5381。

## 12. 歌词自动识别编码并保留逐字毫秒

见[接口清单](endpoints.md)第三节。`fetch_lyric` 保持 `wordLyric` 兼容，同时在 `lyric` 对象里返回
`qrcLines`、`romanLines`，每行与每个词都带 `startMs`/`durationMs` 的原始毫秒值。
歌词字段可能是明文、base64 文本或 QRC hex 密文；组件按可验证的解码结果处理，不能把所有内容一概当 base64。
`[kana:…]` 属于翻译文本元数据，保持明文，不送进 QRC 解密器。

QRC 密钥是 QQ 私有的「类 DES」，不是标准 DES；解密后再取 XML 内的歌词载荷。结构化解析保留词里的空格、方括号、
括号文字以及末尾未计时文本；旧 `wordLyric` LRC 仍使用原有格式转换，其时间精度按 LRC centisecond 表示。

## 13. 空值的两种含义

- **整表读取**（账号列表、收藏）里空列表＝端点形状变了＝**不是答案**，要报错；
  否则会用空列表覆盖宿主缓存里的真数据。
- **歌曲简介**里的空＝这首歌本来就没有简介＝**就是答案**，照常返回。

同一条读取路径上挂着两种相反的规则，判据是"空是不是一种合法的正常状态"。扩展端点层按同一条判据：

- **键不在＝故障**：收藏歌单/专辑/MV（`v_list`/`mvlist`）、关系列表（`List`）、好友（`Friends`）、
  创建的歌单（`v_playlist`）、音乐基因的 `UserInfoCard`、歌手资料的空壳（连 `Name` 都没有）。
- **键在而空＝答案**：评论四个列表、时刻评论、MV/歌曲关联的空映射与空列表、推荐流、
  不喜欢列表（参考模型带 `default_factory=list`）、关系列表翻过末尾的空页。
- **`10007`（没有曲谱）＝答案**，`80092`（歌已在/不在歌单）＝成功，见 §15。

## 13b. 风控码 `2001` 要当成错误

上游被限流时回的是一份"成功但空"的结果集（`code 2001`）。照单全收就会把"被限流"显示成
"搜不到"。组件对 `2000/2001/1000/104401/104400` 一律报错，理由写在错误信息里。

## 14. 限流与熔断在组件里

组件是唯一看得见全部流量的地方，所以两个机制放在这里：按内容类别分桶的固定窗口
（超限**等待**——每次调用都是用户看得见的读取，延迟比失败好），以及熔断（连续失败开路，
半开只放一个探测）。两者都可从宿主配置（`set_rate_limit` / `set_breaker`），
`get_status` 回显生效值。组件是独立进程，**退出即忘**，所以宿主每次拉起后都要重推。

## 15. 签名路（`musics.fcg` 的 `zzc`）与 `musics.fcg` 不是同一条

需要 `zzc` 签名的端点走 `POST musics.fcg?sign=…`：信封、module/method、参数与
`musicu.fcg` 完全相同，只是 URL 上多一个 `zzc`。**服务端按收到的字节校验**，签名对不上回
`2000`——所以发送与签名必须用同一份字节，不能序列化两次。算法的固定下标与异或表属于已验证协议行为（实现见 `src/port/signed.rs`），不得随意改写。

签名路上有两种 comm：

- **账号 comm**（不喜欢家族的读取）——组件默认那套，签名只是多一个 `zzc`；
- **匿名 h5 comm**（`override_comm=True`，乐谱两件、虫虫钢琴档）——`uin` 空、`g_tk` 是
  **字面量 5381**，不是账号的 `g_tk`。`g_tk` 用错不会报错，只会得到空数据。

乐谱相关方法是否走签名路以当前生产实现与 [endpoints.md](endpoints.md) 为准；签名路只改变 URL 的 `zzc`，信封与参数契约保持一致。

**`80092` / `10007` 这类"业务码不是错误"**：`call_with` 把它们收成错误，各领域自己还原。
`10007`（没有曲谱）还原成空答案；`80092`（歌已在/不在歌单）按成功收下——后者是**从错误
文案的括号里解析码**（`上游返回错误（80092）：…`），依赖 `upstream.rs` 的错误格式。

## 16. 扩展端点层里几个键名的陷阱

- **评论回值的列表与分页字段在 `CommentList` 里，`Msg`/`SubCode`/`TotalCmNum` 在它的兄弟上**；
  计数的 `count` 系列在 `response` 里，`cmTabType` 也在兄弟上。按一套形状找齐会两边都丢。
- **`fetch_hot_comments` 的 `PageNum` 是 `page - 1`**（第一页传 0）；这是已验证请求规则，不是笔误。
- **`GetOtherVersionSongs` 只认 `songmid`/`songid`，不认 `mids`/`ids`**（真机核实）。
  参数 `value` 只在数字 ID 与 MID 两种形状之间二选一。
- **MV 地址的 `newFileType`/`fileSize` 是驼峰**，而 MV 详情的上传者字段在上游实际是
  snake_case（`uploader_hasfollow` 等）。候选键两类都列了，但同一字段拼写若随账号变化，
  以哪套为准需要人确认（见[待决清单](pending.md)）。
- **模型里 `has_ldy`→`hasLdy`、`has_qrcx`→`hasQrcx`、`score_mid`→`scoreMid`、`pic_urls`→`picUrls`**
  是照本层规则转的 camelCase，不是上游那套 `hasLDY`/`scoreMID`/`picURLs`；宿主按上游缩写读键名会读空。
- **`fetch_artist_tab` 的 `TabID` 在请求里是字符串**（`wiki`/`song_sing`/…）；
  为兼容既有调用，数字 1 也会映射到对应的 Tab 字符串。
- **`resolve_song_urls` 顶层 `fileType` 属加密类时改打 `GetEVkey`**；项里各自的加密类型
  **不参与**这个判断；这是当前已验证协议行为。该档实测回 `ekey: null`，宿主需自行决定是否降级到普通档。


## 17. 响应编码与类型化契约

`PlaylistBaseWrite`（建/删歌单）与 `PlaylistDetailWrite`（加/删歌）可能返回 GBK 字节，却标注 UTF-8。
组件仅在这两个模块且响应不是有效 UTF-8 时尝试 GBK；其他 JSON 路径继续严格校验。异常 UTF-16 单独代理码以 U+FFFD 表示，合法代理码对及字面量反斜线保持原样。

类型化 FFI 按每个协议方法的命名外壳解包（例如 `detail`、`artistDetail`、`stream`、`tracks`），
外壳缺失时报错，不再反序列化成全空模型。歌词还合并并列的 `wordLyric`。`call_with_platform` 返回原始协议 JSON，保持外壳。

歌手简介请求采用参考的 `singer_mids` 和整数 `1` 开关。助唱、多风格翻译与 AI 词典使用数字 `songID`；助唱请求保留 `needNum:false`。
无多风格翻译或词典时，上游实测返回 `lyrics:null` / `dictList:null`，这两项归一为空数组。
