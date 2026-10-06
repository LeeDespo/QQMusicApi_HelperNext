# AGENTS.md

面向在本仓库工作的自动化代理与人类贡献者的守则。这里只放长期有效的规则与路由；
接口细节在 `docs/`，版本与数字以代码和 `docs/release.md` 为准，不在本文件复制。

## Repository Role

- 本仓库是 **QQMusicApi_HelperNext**：[QQMusicApi](https://github.com/L-1124/QQMusicApi) 的 Rust 移植，
  一个可被多种宿主嵌入的跨平台组件。库即产品：协议调用、请求签名、JSON 解析、凭据、限流、熔断、
  下载引擎托管全部在 `src/`。
- **两个调用面、一个实现**：
  - **typed FFI**：`#[export]` 函数（`src/api.rs` 与 `src/port/`）经 BoltFFI 生成 Swift / Kotlin 绑定；
  - **stdio 子进程**：`src/bin/stdio.rs`，一行一个 JSON（`{"id","method","params"}`）。

  两者都汇入 `methods::dispatch`，行为必须一致，禁止任何一侧自建 HTTP。
- 各平台（macOS / Android / iOS / wasm / Windows / Linux）当前适配到什么程度，以 README 顶部的
  适配度矩阵与 `docs/release.md` 为准；「能生成」不等于「正式支持」。

## Boundary

- 属于本仓库：上游协议实现、签名与平台档案、响应解析、凭据与设备身份存储、限流/熔断、
  aria2 下载引擎托管、FFI 绑定生成配置（`boltffi.toml`）。
- 不属于本仓库：宿主的 UI 与业务逻辑、QQMusicApi 参考实现自身的文档、消费端的构建与更新流程。
- **`src/port/` 契约冻结**：移植层一领域一文件，其中既有文件的既有契约（方法名、参数、回值、
  `#[export]` 包装签名）不允许改动。`src/api.rs`、`src/models.rs`、`src/methods.rs`、`src/catalog.rs`
  的既有内容同为已上线契约。破坏性变更先在 `docs/pending.md` 记录，再走评审。

## Required Reading

按任务路由，开工前先读对应文档（均为相对本仓库根的路径）：

| 任务 | 必读 |
|---|---|
| 任何代码改动 | `docs/architecture.md`（分层、模块职责与禁止事项、边界体检） |
| 新增 / 修改接口 | `docs/endpoints.md` + `docs/parsing.md`，再按 `docs/development.md` 的检查表执行 |
| 动 `#[data]` / `#[export]` | `docs/ffi.md`（绑定生成方式、成套更新规则） |
| 测试与真机验证 | `docs/testing.md`（分层与安全规则）、`docs/pending.md`（已验证范围与边界） |
| 发布相关 | `docs/release.md`（唯一 release 真源） |
| 改动流程与顺序 | `docs/development.md` |

## Source Ownership

- `src/lib.rs` — 组合根：`initialize`/`configure`（首次配置生效后冻结）、`HelperError`。
- `src/api.rs` — 类型化公开 API（`#[export]`）与协议信封到模型的适配。
- `src/models.rs` — `#[data]` 数据模型，camelCase，一份定义同时服务 JSON 与 FFI。
- `src/methods.rs` — 协议层：`METHODS` 方法表、`dispatch`（既有表 → port 层 → 内建方法）、版本常量。
- `src/catalog.rs` / `src/login.rs` / `src/qrc.rs` — 既有端点、扫码登录、逐字歌词。
- `src/upstream.rs` — **唯一**的 QQ 上游 HTTP（`musicu.fcg`、签名 `musics.fcg`、老 fcgi）与 `Platform` 档案。
- `src/guard.rs`（限流 + 熔断）、`src/credential.rs`（凭据）、`src/device.rs`（QIMEI 设备身份，复用共享 agent）、
  `src/aria2.rs`（本地下载引擎，loopback JSON-RPC，不是上游）。
- `src/port/` — 参考实现移植层，一领域一文件。新端点 = 新建 `src/port/<domain>.rs`
  （`METHODS`、`dispatch`、`#[data]` 模型、`#[export]` 包装、测试全部在这一个文件），
  并在 `src/port/mod.rs` **三处登记**：`mod` 列表、`all_methods()`、`dispatch()`。
  漏登记会被模块自带的确定性测试拦下（同名包装检查、跨领域方法名唯一性检查）。

## Development Workflow

1. **先查已有能力**：同一上游端点只允许一个实现；别名或重复能力复用既有入口，不新增方法。
2. 按 `docs/development.md` 的固定顺序完成：实现 → 登记 → 类型化导出 → 生成绑定 → 测试 → 文档同步。
3. **验证闸门按强度顺序**，低级别不过不跑高级别：
   1. `cargo test`（全离线，基线必须绿）；
   2. `boltffi generate swift` 与 `boltffi generate kotlin`（动过 `#[data]`/`#[export]` 后必须成套重跑）；
   3. `scripts/port-smoke.sh`（真机只读；环境类错误记 skip，不算失败）。
4. 提交前过一遍 Completion Checklist。

## Public API-FFI Rules

- `#[data]` / `#[export]` 是公开契约：**字段只追加、不重排、不改名**。FFI 按字段顺序编码，
  Swift/Kotlin bindings 与 native library 必须由同一版本成套生成、打包与发布，不能新旧混用（详见 `docs/ffi.md`）。
- `api_surface_matches`（`src/api.rs` 的测试）强制协议方法表与类型化包装一一对应；
  新增方法必须同时落 `METHODS` 与 `#[export]` 两处，否则测试失败。
- typed FFI 与 stdio 是同一实现的两张脸：对 QQ 上游的 HTTP 只存在于 `src/upstream.rs`。
  任何新代码不得自带 HTTP agent、限流器或熔断器；需要上游就走共享的 `Upstream`
  （port 层统一入口 `port::call`，见 `src/port/mod.rs`）。
- 不得修改 `src/port/` 既有契约（见 Boundary）。

## Testing Safety

- 默认只跑离线测试。真实账号与网络测试必须显式选择进入（Rust 侧 `#[ignore]`，写侧 `--execute-writes`）。
- 真机冒烟只读：`scripts/port-smoke.sh` 只分发脚本内白名单方法；登录流程永不测试；环境类错误记 skip。
- **写接口铁律**：明确授权、可逆、前后快照、复原确认、失败不自动扩大。完整规则见 `docs/testing.md`。
- 凭据与密钥（cookies、`qm_keyst`、`musickey`、`encrypt_uin` 等）不进 git、不进日志、不进文档与测试报告；
  stdio 回包与诊断日志同样不得出现凭据。

## Documentation Rules

- 文档在 `docs/`，互相以相对路径引用。`docs/endpoints.md` 与 `docs/parsing.md` 是接口契约记录，
  改接口必须同步；设计取舍、未决事项与验证边界进 `docs/pending.md`。
- 历史验证报告属于取证记录，不是现行规范，不作为改代码的依据。
- 本文件与各文档只放规则和路由；易变的细节（版本号、方法数、测试数、产物清单）以代码和
  `docs/release.md` 为准，不复制多份。

## Release

- 发布事务的唯一真源是 **`docs/release.md`**；不得创建根级 `release_plan.md` 或其他第二真源。
- 硬规则速记：Git tag = `Cargo.toml` version = Release 版本；clean tree 才能发布；
  Android Kotlin binding 与四 ABI `.so` 是不可拆配的成套资产；未经真实验证的平台不进 Release
  （wasm、Apple FFI、Windows、Linux 现阶段均为「不发布」）。
- `.github/workflows/release.yml` 尚未实现；实现规格与前置项见 `docs/release.md`。

## Completion Checklist

- [ ] `cargo test` 绿；动过 `#[data]`/`#[export]` 的，Swift/Kotlin 绑定已成套重新生成。
- [ ] 只对本次改动文件跑过 `rustfmt --edition 2021 <文件>`；**没有**全仓跑 `cargo fmt`。
- [ ] `src/port/` 既有契约未被改动；新端点已在三处登记并被确定性测试覆盖。
- [ ] `docs/endpoints.md`、`docs/parsing.md`、`docs/pending.md` 与实际行为一致。
- [ ] 没有把凭据或密钥写进任何文件、日志或测试报告。
- [ ] 涉及发布语义的改动已同步 `docs/release.md`。
