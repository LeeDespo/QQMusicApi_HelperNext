# HelperNext 接续完成与验证报告（2026-10-04）

## 结论

已完成原 `.zcode` 移植工作流的非登录接口接续验收，新增 8 个轻量遗漏接口，并修复已有歌手简介、FFI 解包、资源 ID 覆盖及写接口响应编码问题。
真实账号测试通过，实际写操作已核验复原。所有登录、扫码与截图测试均未执行。
原工作流延期的私信、COS 上传和 MQTT 扫码仍未实现，不能将本次结果称作 QQMusicApi 全部能力已移植。

## 查阅范围与来源

- HelperNext `.zcode/workflow-drafts/port-endpoints-draft.ts` 与 `.zcode/workflow-runs/`：原领域任务、参数规范和验收方式。
- 参考快照 `dist/reference/QQMusicApi/qqmusic_api/modules/`、对应模型与测试输入。
- 原 `docs/pending.md`、接口与解析说明，以及 Music_app `.zcode/research/` 的接口对应、FFI 机制和前次验证报告。
- 仓库开始时已存在前 agent 的大量未提交改动，本报告描述本次接续工作，不把整个 Git diff 当作本次新增。没有提交或推送。

## 完成的功能

| 功能 | 协议方法 |
|---|---|
| 新碟上架与分页 | `fetch_new_albums` |
| 指定用户喜欢的歌曲 | `fetch_user_liked_songs` |
| 助唱标注可用性 | `fetch_singing_annotations` |
| 多风格翻译歌词与 QRC 解密 | `fetch_multi_style_lyrics` |
| AI 词典可用性与读取 | `has_ai_dictionary`, `fetch_ai_dictionary` |
| 收藏/取消外部公开歌单 | `fav_playlist`, `unfav_playlist` |

这些方法均已登记分发、类型化导出及 Swift/Kotlin 绑定。

### 修复

- **歌手简介**：`GetSingerDetail` 使用准确的 `singer_mids` 与整数开关，从匹配歌手的 `ex_info.desc` 读取简介，必要时取 wiki 内容；不再恒为空。
- **FFI 响应解包**：按协议外壳读取详情、列表、歌词与播放地址；保留总数、合并逐字歌词，缺少预期外壳时报错。修复歌曲 `genreTags` 与模型 `genre` 的匹配。
- **本地控制的 FFI 路径**：限流、熔断和 aria2 方法接入公共分发器；宿主默认平台配置生效。`call_with_platform` 保持原始协议 JSON 形状。
- **子进程资源 ID**：顶层 `id` 继续关联请求，响应自身的资源 ID 放入 `resultId`。FFI 模型的资源字段仍叫 `id`。
- **GBK 响应**：真实写入发现 `PlaylistBaseWrite` 和 `PlaylistDetailWrite` 会返回 GBK 字节却标注 UTF-8。仅这两族且响应不是有效 UTF-8 时进行 GBK 解码；其他路径保持严格校验。
- **错误成功判据**：收藏写入要求有效结果码，失败 ID 列表不能默默丢掉损坏项；删除评论遵循参考：缺失 `SubCode` 默认为成功，但存在的错误字段不能伪装成功；实际清理通过评论读回核验；清空不喜欢的预检失败/缺失 Token 时不发送删除。
- **可选歌词内容**：真实无内容响应 `lyrics:null`、`dictList:null` 归一为空数组；错误类型仍报错。
- **测试工具**：只读脚本具有严格允许名单、超时、JSON/模型验证及明细；拒绝用户误传写方法。写测试处理丢失创建回复、资源归属、迟到回复和独立清理失败，避免删除原有歌单或遗留测试资源。

## 验证结果

| 验证 | 结果与边界 |
|---|---|
| 普通 Rust 测试 | 231 个库测试 + 2 个子进程测试 + 1 个文档测试通过，0 失败；默认 7 个联网用例被忽略 |
| 类型化真实读取 | 1 个显式联网集成测试通过：详情、歌手简介/作品、专辑曲目、榜单/电台、新歌、歌词、播放地址、账号列表和原始协议契约 |
| 清空不喜欢预检 | 1 个显式联网 Token 预检通过，不发送全量删除命令 |
| 只读接口 | 52 个不同方法，76 个用例通过，0 失败、0 跳过；含搜索 10 类型、续页、乐谱 3 请求类型和附加歌词有/无内容样本 |
| 可逆写入 | 最终 5 组通过；13 个不同写方法、16 次调用，复原核验通过 |
| 写测试失败保护 | 4 个 Python 离线测试通过，覆盖原歌单归属、丢失回复、迟到回复、清理失败及跳过统计 |
| 跨平台绑定 | `boltffi generate swift`、`boltffi generate kotlin` 均成功；确认两端包含全部 8 个新接口 |
| 格式/协议检查 | 修改过的 Rust 文件格式化，`git diff --check`、Python 语法与只读允许名单检查通过 |

默认忽略测试并非遗漏执行：本轮显式运行类型化读取和 Token 预检；另外两项签名/乐谱读取由只读脚本覆盖。微信取码/查询及设备握手独立探针未运行。没有执行宿主 UI、真机打包或发布。

## 写接口报备与复原

| 实际调用 | 写入/删除的内容 | 复原证据 |
|---|---|---|
| `create_playlist` / `delete_playlist` | 带唯一时间戳的 `HelperNext接口测试-*` 临时歌单；中途另建编码诊断歌单 | 创建歌单目录 ID 集合恢复基线；所有本次测试歌单删除 |
| `add_playlist_songs` / `remove_playlist_songs` | 临时歌单加入并移除歌曲 361947418；重复增删验证幂等 | 加入后只有一份，移除后为空，最终父歌单删除 |
| `add_comment` / `delete_comment` | **仅在本次临时歌单**发布“HelperNext接口连通性测试，完成后删除。” | 删除前读取可见、删除后读取不可见，父歌单也删除；未向其他用户发送私信 |
| `fav_album` / `unfav_album` | 临时收藏专辑 28791467 | 收藏后读取可见；取消后专辑 ID 集合与基线一致 |
| `set_liked`（true / false） | 临时喜欢歌曲 361947418 | 喜欢后读取可见；取消后可见歌曲 ID 集合与基线一致 |
| `add_dislike` / `cancel_dislike` | 临时不喜欢歌曲 361947418 | 添加后读取可见；取消后不喜欢 ID 集合与基线一致 |
| `fav_playlist` / `unfav_playlist` | 临时收藏公开歌单 7039749142 | 收藏后读取可见；取消后收藏歌单 ID 集合与基线一致 |
| `CancelAllDislike` Token 预检 | 请求 `ISOnlyGetToken:true`，不发送删除命令 | 预检成功，既有条目未删除，Token 未输出 |

只读明细：[只读测试结果](evidence/2026-10-04/read-smoke-2026-10-04.json)。

最后成功写入明细：[最终写测试结果](evidence/2026-10-04/write-smoke-final-2026-10-04.json)。
中途编码失败时，远端操作已生效而解析失败；通过唯一名称读回找到了资源并清理。
早期操作与人工诊断也保留在 [初始记录](evidence/2026-10-04/write-smoke-initial-2026-10-04.json)、[恢复记录](evidence/2026-10-04/write-smoke-recovery-2026-10-04.json)、[中间记录](evidence/2026-10-04/write-smoke-2026-10-04.json)、[首次成功记录](evidence/2026-10-04/write-smoke-first-success-2026-10-04.json)以及[删除评论契约复核](evidence/2026-10-04/write-smoke-comment-contract-2026-10-04.json)，不要把失败回复当成“没有写入”。
删除评论允许成功响应省略 SubCode，最后一次验证额外通过前后读回确认资源消失。
以上复原针对成员集合和临时资源；没有删除原有收藏、喜欢或不喜欢条目来测试，保留其原始时间与顺序。

## 账号与验证边界

第一次凭据因账号资产接口明确返回 1000 而按用户要求中止；用户补充登录后，收藏专辑与不喜欢列表确认新会话有效，再恢复任务。
VIP 公开信息返回成功不能证明会话有效。凭据没有打印、纳入报告或测试固定数据。

- 全量 `clear_dislike_songs` 未执行：已有 9 个原始不喜欢条目，重建会改变时间和顺序，故只验证预检、参数与失败保护。
- 指定用户喜欢接口用本账号的显式 `euin` 验证；没有额外他人账号/隐私限制样本。
- 部分内容分支随曲目而为空；真实读取通过不能替代所有曲目的授权与非空样本保证。
- 私信/COS/手机客户端 MQTT 属于原工作流延期模块；剩余边界见 [pending.md](../pending.md)。
- 全程没有查看截图、测试登录或发送真实短信。未更新或发布宿主 App。

## 复跑入口

```sh
cargo test
python3 -B tests/test_write_smoke.py
# 真读取，不会执行登录或写操作；指定用户喜欢读取需通过环境选择目标 euin。
scripts/port-smoke.sh <helper-dir>
# 类型化读取：配置已有凭据目录，不执行登录。
QQMUSIC_HELPER_NEXT_DIR=<helper-dir> cargo test --test live_typed -- --ignored
# 仅预检 Token，不清空账号数据。
QQMUSIC_HELPER_NEXT_DIR=<helper-dir> cargo test --lib clear_dislike_preflight_without_deletion -- --ignored
# 显式启用写测试；先重新构建 binary，再执行自动清理与结果记录。
cargo build --bin qqmusic-helper-next
python3 scripts/port_write_smoke.py --execute-writes --report <result.json>
```
