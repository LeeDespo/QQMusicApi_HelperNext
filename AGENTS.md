# AGENTS.md

本文件是**本仓库**的规则与阅读路由。通用工作方式（任务自适应、最小改动、提交与汇报纪律、
文档哲学）由全局 `~/.codex/AGENTS.md` 规定，不在这里重复。

## Repository Role

- **QQMusicApi_HelperNext** 是 [QQMusicApi](https://github.com/L-1124/QQMusicApi) 的 Rust 移植：
  一个可被多种宿主嵌入的跨平台组件。库即产品——协议调用、请求签名、平台档案、响应解析、凭据、
  限流与熔断、下载引擎托管全部在 `src/`。
- **两个调用面、一个实现**：
  - **typed FFI**：`#[export]` 函数（`src/api.rs` 与 `src/port/`）经 BoltFFI 生成 Swift / Kotlin 绑定；
  - **stdio 子进程**：`src/bin/stdio.rs`，一行一个 JSON（`{"id","method","params"}`）。

  两者都汇入 `methods::dispatch`，行为必须一致，禁止任何一侧自建 HTTP。
- 各平台（macOS / Android / iOS / wasm / Windows / Linux）适配到什么程度，以 README 顶部的
  适配度矩阵与 `docs/RELEASING.md` 为准；「能生成」不等于「正式支持」。

## Repository Boundary

- 属于本仓库：上游协议实现、签名与平台档案、响应解析、凭据与设备身份存储、限流/熔断、
  aria2 下载引擎托管、FFI 绑定生成配置（`boltffi.toml`）。
- 不属于本仓库：宿主的 UI 与业务状态（播放、MediaStore、下载台账）、QQMusicApi 参考实现自身的文档、
  消费端的构建与更新流程。
- **`src/port/` 契约冻结**：移植层一领域一文件，其中既有文件的既有契约（方法名、参数、回值、
  `#[export]` 包装签名）不允许改动。`src/api.rs`、`src/models.rs`、`src/methods.rs`、`src/catalog.rs`
  的既有内容同为已上线契约。破坏性变更先在 `docs/pending.md` 记录，再走评审。

## Sources of Truth

按主题各有一处真源，同一事实不写两遍（方法数、测试数、版本号、产物清单以代码为准）：

| 主题 | 真源 |
|---|---|
| 分层、模块职责、禁止事项、边界体检 | `docs/architecture.md` |
| 当前接口事实（上游 module/method、参数、回值） | `docs/endpoints.md` |
| 长期解析陷阱与必读结论 | `docs/parsing.md` |
| 公开 FFI 契约与成套更新规则 | `docs/ffi.md` |
| 测试分层与真机安全 | `docs/testing.md` |
| 未完成能力与验证边界 | `docs/pending.md` |
| 发布流程与资产规则 | `docs/RELEASING.md` |
| 历史报告与取证记录 | `docs/history/` |
| 测试数据与真实响应样本 | `tests/fixtures/` |

## Required Reading

开工先读本文件，再按任务读对应文档（均为相对本仓库根的路径）：

| 任务 | 必读 |
|---|---|
| 任何代码改动 | `docs/architecture.md`（分层、模块职责与禁止事项） |
| 新增 / 修改接口 | `docs/endpoints.md` + `docs/parsing.md`，再按 `docs/development.md` 的检查表执行 |
| 动 `#[data]` / `#[export]` / `boltffi.toml` | `docs/ffi.md`（绑定生成方式、成套更新规则） |
| 测试与真机验证 | `docs/testing.md`（分层与安全规则）、`docs/pending.md`（已验证范围与边界） |
| 改动流程与顺序 | `docs/development.md` |
| 发布、打包、资产、tag | `docs/RELEASING.md`（发布流程只在那一处维护） |
| 排查历史行为 | 仅在当前文档不足时查 `docs/history/` |

## Project Invariants

- **一个上游端点一个实现**：别名或重复能力复用既有入口，不新增方法。新增方法必须同时落
  `METHODS` 与 `#[export]` 两处，否则 `api_surface_matches` 测试失败。
- **`#[data]` / `#[export]` 是公开契约**：字段只追加、不重排、不改名。Swift/Kotlin bindings 与
  native library 必须由同一版本成套生成、打包与发布，不能新旧混用（详见 `docs/ffi.md`）。
- **对 QQ 上游的 HTTP 只存在于 `src/upstream.rs`**：任何新代码不得自带 HTTP agent、限流器或熔断器；
  需要上游就走共享 `Upstream`（port 层统一入口 `port::call`）。
- **新端点 = 新建 `src/port/<domain>.rs`**，`METHODS`、`dispatch`、`#[data]` 模型、`#[export]` 包装与
  测试都在该文件内，并在 `src/port/mod.rs` 三处登记：`mod` 列表、`all_methods()`、`dispatch()`；
  漏登记会被模块自带的确定性测试拦下（同名包装检查、跨领域方法名唯一性检查）。
- **分页按上游原始行推进**：无 MID 等不可用行同样占上游位置；组件回传 `nextOffset` 时用它，
  缺省才回退原始行数——不能按过滤后的条数或请求 `limit` 推进。
- **凭据只进不出**：cookies、`qm_keyst`、`musickey`、`encrypt_uin` 等不进 git、日志、文档与测试报告；
  stdio 回包与诊断日志同样不得出现凭据。

## Test / Safety Exceptions

- 默认只跑离线测试 `cargo test`。真实账号与网络测试必须显式进入（Rust 侧 `#[ignore]`，
  写侧 `--execute-writes`）；登录流程、扫码与截图不在测试范围。
- 真机只读冒烟 `scripts/port-smoke.sh` 只分发脚本内白名单方法；环境类错误（登录过期、风控）记 skip、
  不算失败；未设 `PORT_SMOKE_EUIN` 时相关用例可见地跳过。
- **写接口铁律**：明确授权、可逆、操作前后有快照、复原后确认、失败不自动扩大；禁止为「确认接口能用」
  而批量操作真实账号。完整规则见 `docs/testing.md`。

## Quality Gates

按强度顺序执行，低级别不过不跑高级别：

1. `cargo test`（全离线，基线必须绿）；
2. `boltffi generate swift` 与 `boltffi generate kotlin`（动过 `#[data]`/`#[export]` 后必须成套重跑）；
3. `scripts/port-smoke.sh`（真机只读）。

另有两条仓库级护栏：

- `bash scripts/check_repository_rules.sh`：仓库结构检查（`src/` 洁净度、产物不入库、release 真源唯一、
  无凭据字面量）。
- 格式：**禁止全仓跑 `cargo fmt`**（仓库基线不是逐文件格式化的，全仓跑会把无关文件重排出一大堆 diff）；
  只对本次改动的文件跑 `rustfmt --edition 2021 <文件>`。

接口行为变了就同步对应真源（`docs/endpoints.md` / `docs/parsing.md`），验证缺口进 `docs/pending.md`，
动过公开契约就同步 `docs/ffi.md`。

## Release

版本号、tag、发布资产、打包、校验和、支持的发布目标与 release CI，一律读并遵守
`docs/RELEASING.md`；本文件不重复发布流程，也不得创建第二个 release 真源。
