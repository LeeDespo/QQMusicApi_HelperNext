# NeuMusic（安卓宿主）接入验证（2026-10-05）

状态：**已完成**。本文件保留当轮带日期的取证细节；接入的取舍与分工由宿主仓库（NeuMusic）
自己的接入报告维护，本仓库不复述宿主内部实现。相关现行规则见文末「沉淀去向」。

## 当轮做了什么

- 本轮新增的分页 / 轮播批次 / 写回执方法在真账号上通过只读验证；写接口仅 `set_liked_by_id`
  一次：对未喜欢的歌曲 `songId=272125057`（`mid=0013WPvt4fQH2b`）执行
  `music.musicasset.PlaylistDetailWrite / AddSonglist` 一次、随后 `DelSonglist` 一次，
  两次均获成功回执，操作前后喜欢列表 ID 集合、服务器总数与原始分页行数一致（复原确认）。
  其余写接口沿用 2026-10-04 轮（[接续验证报告](2026-10-04-continuation-verification.md)）的结论；
  失败不自动重试（含 `1000` 限流码）。
- Android 四 ABI 编译与产物检查全部成功；16 KB page-size（ELF LOAD 段对齐）检查通过；
  emulator-5554（arm64）以保留数据方式安装启动 debug APK，其余三 ABI 未做设备运行验证。
- 现有 macOS 安装**未替换**：继续使用 0.1.0 产物；0.2.0 bindings 与 native 库需成套替换。
- 宿主侧组件产物由同次 BoltFFI 打包生成，记录了源码 revision、逐文件 SHA-256 与 sourceSha256
  （宿主 `app/helpernext/manifest.json`），并支持从 manifest 记录的远端 revision 精确重建。

## 沉淀去向（现行规则所在处）

- FFI 0.2.0 成套更新规则 → [../ffi.md](../ffi.md)
- 电台轮播批次 `fetch_radio_track_batch`（`total` 恒为 `null`）→ [../endpoints.md](../endpoints.md)
- `set_liked_by_id` 回执语义（`success` / `code` / `throttled`）→ [../endpoints.md](../endpoints.md)
- 未完成能力与验证边界 → [../pending.md](../pending.md)
