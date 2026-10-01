# 解析要点

上游响应的形状不统一，这一份记录那些**不直观、但必须照做**的地方。
每一条都是结论；推导过程在提交信息里，不在这里。

## 1. 平台档案按接口选，选错只回空数据

`comm` 有两套档案，用途不同（见[接口清单](endpoints.md)第一节）：

- **web**（`cv 4747474 / ct 24 / platform "yqq.json" / needNewCode 1`）——账号列表、曲库详情、榜单、电台、新歌。
- **android**（带设备身份与设备会话）——搜索、取流、收藏写、推荐流、歌手资料、封面匹配。

用错档案不会报错：搜索会回 `meta.sum = 0`，取流会回 `104003`。所以"某个接口今天突然没数据了"
先看档案，再看参数。

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

- 关注歌手的列表键是 **`List`**（大写 L）。QQMusicApi 的模型里叫 `users`，照模型去找会以为"没有这个能力"。
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
| `artist` | `singer[].name` 用 `", "` 连接 |
| `album` / `albumMid` | `album.name` / `album.mid` |
| `albumId` | `album.id` / `album.albumId`（**数字**，专辑页要用它） |
| `imageURL` | 由 `albumMid` 拼 `T002R800x800M000<mid>.jpg` |
| `duration` | `interval` / `duration`（秒） |
| `payPlay` | `pay.pay_play`（嵌套，不是扁平键） |

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

## 12. 逐字歌词要自己解密

见[接口清单](endpoints.md)第三节。要点：`qrc:1` 时 `lyric`/`trans` 都是 **hex 密文**（不是 base64、
也不是明文），解出来是 zlib；**判断 hex 还是 base64 要靠内容**，把密文当 base64 解会得到乱码，
表现成"整首歌词加载不出来"——这类错误不会抛异常，只会渲染出垃圾。

密钥是 QQ 私有的「类 DES」，不是标准 DES；移植时不要拿通用 DES 库硬套，盒子表和密钥位序都得照抄。

## 13. 空值的两种含义

- **整表读取**（账号列表、收藏）里空列表＝端点形状变了＝**不是答案**，要报错；
  否则会用空列表覆盖宿主缓存里的真数据。
- **歌曲简介**里的空＝这首歌本来就没有简介＝**就是答案**，照常返回。

同一条读取路径上挂着两种相反的规则，判据是"空是不是一种合法的正常状态"。

## 13. 风控码 `2001` 要当成错误

上游被限流时回的是一份"成功但空"的结果集（`code 2001`）。照单全收就会把"被限流"显示成
"搜不到"。组件对 `2000/2001/1000/104401/104400` 一律报错，理由写在错误信息里。

## 14. 限流与熔断在组件里

组件是唯一看得见全部流量的地方，所以两个机制放在这里：按内容类别分桶的固定窗口
（超限**等待**——每次调用都是用户看得见的读取，延迟比失败好），以及熔断（连续失败开路，
半开只放一个探测）。两者都可从宿主配置（`set_rate_limit` / `set_breaker`），
`get_status` 回显生效值。组件是独立进程，**退出即忘**，所以宿主每次拉起后都要重推。
