# 测试：怎么安全验证

本文是验证的操作手册：四层测试怎么跑、真机与真实账号的强规则、凭据纪律。
闸门顺序与改动流程见 [development.md](development.md) 与 [AGENTS.md](../AGENTS.md)；
当前已验证到哪、明确没验证什么，见 [pending.md](pending.md)。

## 1. 四层总览（按强度递增）

| 层 | 内容 | 网络/账号 | 授权方式 |
|---|---|---|---|
| L1 | `cargo test` | 全离线 | 默认执行 |
| L2 | `python3 tests/test_write_smoke.py`（写复原逻辑的离线失败路径） | 全离线 | 默认执行 |
| L3 | `tests/live_typed.rs`（`#[ignore]`）+ `scripts/port-smoke.sh`（真机只读） | 真实网络/账号，**只读** | 显式：`#[ignore]` 测试需 `cargo test -- --ignored` 并设 `QQMUSIC_HELPER_NEXT_DIR`；冒烟脚本手动执行 |
| L4 | `scripts/port_write_smoke.py`（真实账号可逆写） | 真实网络/账号，**会写** | 显式 `--execute-writes` 旗标 |

验证闸门按强度顺序：`cargo test` → `boltffi generate swift` / `boltffi generate kotlin` →
`scripts/port-smoke.sh`。低级别不过，不跑高级别。

## 2. L1：`cargo test`（全离线，基线必须绿）

- 不需要网络、账号、凭据或辅助进程；任何提交前必跑。
- 钉住关键行为的几组测试：
  - **QRC 已知答案向量**：`src/lib.rs` 的 `qrc_vector_check`（向量文件被 `src/lib.rs`、
    `src/catalog.rs`、`src/port/library_extra.rs` 三处 `include_str!` 引用）——解密字数、
    XML 结构、逐字解析、JSON 序列化、word-LRC 全链路；
  - **port 层确定性测试**（`src/port/mod.rs`）：每个方法必须有同名 `#[export]` 包装且在
    `METHODS` 与 `dispatch` 两处出现；方法名跨领域唯一。漏登记在这里爆；
  - **`api_surface_matches`**（`src/api.rs:555`）：协议方法表与类型化包装一一对应（含别名表）；
  - **stdio 协议测试**（`src/bin/stdio.rs`）：`id` 关联、资源 `id` → `resultId` 的搬运；
  - 配置冻结、信封解析、guard 窗口等各自模块内测试。
- `#[ignore]` 的联网用例不会在默认 `cargo test` 里执行；且 `#[ignore]` 不止 `tests/live_typed.rs` 一个：
  `src/` 内还有设备握手（`device.rs` `live_qimei_handshake`）、登录扩展探针（`port/login_extra.rs`
  `live_wx_qrcode` / `live_wx_status`）、签名路探针（`port/signed.rs` `live_signed_route` /
  `live_sheet_route`）与清空不喜欢预检（`port/user_asset.rs`
  `clear_dislike_preflight_without_deletion`）。显式执行 `cargo test -- --ignored` 会把这组联网
  用例一并选中，需要凭据目录与真实网络，按第 4、5 节的规则对待。

## 3. L2：写复原逻辑的离线失败路径

```sh
python3 tests/test_write_smoke.py
```

- 无辅助进程、无网络、无凭据：脚本把 `scripts/port_write_smoke.py` 当源码加载，
  用 `FakeSession` 模拟辅助进程，专测**失败路径下的复原逻辑**；
- 钉住的行为：失败的创建**绝不删除原有歌单**（不拥有就不清理）；创建成功但回包丢失时，
  按**精确名字回读**找到测试歌单再删；超时的写请求不得吞掉后续清理请求的响应；
  一处清理失败不阻断其余清理；复原要经 `verified` 确认。

## 4. L3：真机只读验证

### 4.1 类型化入口的实读测试

```sh
QQMUSIC_HELPER_NEXT_DIR=<凭据目录> cargo test --test live_typed -- --ignored
```

- `tests/live_typed.rs` 标了 `#[ignore = "requires QQMUSIC_HELPER_NEXT_DIR and real network/account"]`；
- 覆盖 FFI 宿主实际调用的类型化入口（目录、榜单、歌词、取流、账号读取、guard 状态、
  原始 platform 契约）；本地配置类调用（限流/熔断）进程内生效，不动远端账号；
- 登录流程与账号写入不在此层。

### 4.2 只读冒烟脚本

```sh
scripts/port-smoke.sh [凭据目录] [方法 ...]     # 不给方法 = 跑全部白名单
```

规则（来自 `scripts/port-smoke.sh` 与 `scripts/port_read_smoke.py`，改脚本前先读）：

- **显式白名单**是唯一分发依据（脚本内 `PARAMS` 基表加 `PARAMS.update` 补充，规模与名单以
  `scripts/port_read_smoke.py` 为准）；名单外的方法在构建前就被拒绝，
  不得从名字推断「看起来安全」就放行；
- **登录操作永不测试**（`port-smoke.sh` 头注释明文）；
- 环境变量：
  - `PORT_SMOKE_EUIN`：`fetch_user_liked_songs` 需要的显式加密 UIN；未设置时对应用例记**可见 skip**，不伪装通过；
  - `PORT_SMOKE_TIMEOUT_SECONDS`：单调用超时，1..120，默认 30；
  - `PORT_SMOKE_SKIP_BUILD=1`：跳过构建直接用已有二进制（默认构建限时 180 秒）；
- **环境类错误记 skip，不算失败**：网络不通、凭据过期、风控等环境原因归入 `skips`；
  但形状错误、断言失败是真实 `failed`——脚本对「环境问题」与「坏样本/坏请求」有明确判据，不要放宽；
- 汇总 JSON 以 `SMOKE_RESULT` 为前缀输出，含 `passed/failed/skipped` 与明细，可作为取证记录留档
  （留档前按第 6 节自查凭据）。

## 5. L4：真实账号可逆写

```sh
cargo build --bin qqmusic-helper-next
python3 scripts/port_write_smoke.py --execute-writes --report <结果.json>
```

**强规则**（`scripts/port_write_smoke.py` 头注释与实现，逐条对照过）：

1. **明确授权**：不带 `--execute-writes` 绝不执行任何写；报告路径显式给定；
2. **可逆**：只动「新增的成员关系」和「唯一命名的测试歌单」；**既有我喜欢/收藏/不喜欢条目永不因测试删除**；
3. **前后快照**：写前全量拉取我喜欢、收藏、不喜欢列表作为基线，写后核对（快照有页数上限，
   超限视为快照不完整而失败，不静默继续）；
4. **复原确认**：每个写操作都有对应的清理与回读验证；清理失败如实记 `failed`；
5. **不自动扩大**：失败不自动重试（含 `1000` 限流码）；写请求超时记 `unknown` 并要求人工核对远端状态，
   绝不把「没收到回复」当成「没写进去」；
6. 登录流程、扫码、截图不在本层（历轮验证均按此排除，见 [pending.md](pending.md)）。

## 6. 凭据纪律

- 凭据与密钥（cookies、`qm_keyst`、`musickey`、`encrypt_uin` 等）不进 git、不进日志、
  不进文档、不进测试报告；
- 组件侧已保证：凭据读写只在 `src/credential.rs`，stdio 回包与 stderr 日志不含凭据内容；
- 测试侧自查：任何要留档的 JSON/日志，先扫一遍敏感键再入库。既有 7 个冒烟取证 JSON
  （`docs/history/evidence/2026-10-04/*-smoke-2026-10-04.json`）复核过 `qm_keyst/qimei/cookie/token`
  全部 0 命中——它们是聚合结果（passed/failed/writes/restorations），因此可以作为历史取证留库。

## 7. 失败怎么办

- L1/L2 失败：修到绿再继续，没有「带病前进」；
- L3 环境类 skip：如实记录环境原因；形状/断言失败按真实 bug 走 [pending.md](pending.md) 与修复流程；
- L4 任何意外：先人工核对远端账号状态，再决定补偿动作；测试脚本不会替你做超出快照的清理。
