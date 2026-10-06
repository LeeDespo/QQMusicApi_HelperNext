# Changelog

本文件记录**对外可见**的变化：接口 / FFI / stdio 协议、模型字段、行为、平台支持与发布资产。
内部重构、一次性验证过程与历史审计不写在这里（见 `docs/history/`）。

格式参考 [Keep a Changelog](https://keepachangelog.com/)，版本遵循 Semantic Versioning；
每次发布时把 [Unreleased] 落为版本条目（流程见 [docs/RELEASING.md](docs/RELEASING.md)）。

## [Unreleased]

### Added

- 根 `NOTICE`：第三方来源、许可证关系与非官方关系声明；macOS / Android 发布包随包携带。
- `CHANGELOG.md` 与文档门户 `docs/README.md`。
- 仓库护栏（`scripts/check_repository_rules.sh`）新增：活文档相对链接检查、
  活文档禁用宿主本地路径、Cargo.toml / README / LICENSE 许可证表述一致性。

### Changed

- `boltffi.toml` 的 wasm target 关闭（`enabled = false`）：该平台未适配、未验证，
  配置开启容易被误读为受支持。
- README 把项目用途立场（非商业等）与 GPL 软件许可证分开表述。

## [0.2.0] - 2026-10-06

首个正式 Release：tag `v0.2.0` 经 CI（validate → macOS → Android → release）构建并发布。

### Added

- QQ 音乐协议内核：module/method/params、请求信封、`web` / `android` 平台档案、`zzc` 签名、
  设备身份（QIMEI）、凭据持久化、按内容类别分桶的限流与熔断。
- 移植层端点（评论、MV、歌曲资产、用户关系、收藏写、推荐、搜索扩展、电台、喜欢等）；
  列表分页偏移按上游原始行推进，歌曲/专辑页回传 `total` 与 `nextOffset`。
- 逐字歌词：QRC 毫秒级时间轴、roma 与 kana（`fetch_multi_style_lyrics`）。
- 六档音质 `resolve_song_urls`，逐档回执（成功 / 业务码 / 降级建议）。
- `set_liked_by_id` 数字歌曲 ID 写入与结构化回执（`success` / `code` / `throttled`）。
- stdio 子进程协议与 `--version`（读取任何宿主配置之前回答组件与协议版本）。
- 发布流水线：`scripts/release/` 打包脚本与 tag 触发的 release CI；
  发布资产：macOS ARM64 stdio 包、Android BoltFFI 四 ABI 包、`release-manifest.json`、
  `SHA256SUMS`、`THIRD-PARTY-LICENSES.txt`。

### Changed

- **FFI 0.2.0**：模型 wire 布局与 0.1.0 不兼容（字段只追加、按顺序编码）；
  `AlbumPage` 追加 `nextOffset`。Swift/Kotlin bindings 与 native library 必须成套更新，
  不能与 0.1.0 产物混用。

### Fixed

- `wordLyric` 在 QRC 可解密但解析为 0 行时回 `null`（旧契约），不再回退为非 LRC 文本。
- `poll_login` 读取共享存储前先冻结配置；`set_liked_by_id` 尊重显式指定的 `platform`；
  导入的加密 uin 存储前去除首尾空白。

## [0.1.0]

未正式发布的内部版本：macOS 宿主通过本地构建的 stdio 二进制与 0.1.0 Swift 绑定消费，
没有 tag 与 GitHub Release。升级到 0.2.0 时 bindings 与 native 库必须成套替换。
