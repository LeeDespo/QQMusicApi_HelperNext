# AGENTS.md

本文件只负责**本仓库的身份、边界、真源路由与质量门**。通用工作方式由全局 / root AGENTS 负责；
实现细节、历史记录和发布步骤不要复制到这里。

## Repository Role

QQMusicApi_HelperNext 是一个面向 QQ 音乐互操作的 Rust 组件。产品本体是本仓库的 Rust 内核，
对外提供两种调用面：

- stdio JSON 子进程：`src/bin/stdio.rs`
- BoltFFI typed API：`src/api.rs` 与 `src/port/`

两者必须共享同一套生产实现。QQMusicApi 是协议研究与历史来源之一，不是运行时依赖，也不决定本仓库现行架构。

## Repository Boundary

**属于本仓库**

- QQ 上游协议调用、签名、平台档案与响应解析；
- 凭据 / 设备身份、限流 / 熔断、Aria2 托管；
- Rust 公共 API、stdio 协议、BoltFFI 配置；
- 本组件自己的测试、打包、Release 与第三方许可证履约。

**不属于本仓库**

- 任何消费端的 UI、业务状态、构建流程、升级脚本或安装状态；
- 消费端的 `helpernext.lock.json`、原子替换目录、补丁重建流程；
- 未跟踪的本地参考快照、agent 工作目录、施工计划；
- QQMusicApi 自身的文档副本或消费端实现知识库。

历史来源与一次性验证只允许留在 `docs/history/`，不得作为现行真源。

## Sources of Truth

| 主题 | 真源 |
|---|---|
| 组件版本 | `Cargo.toml package.version`；代码通过 `CARGO_PKG_VERSION` 读取 |
| 架构、模块职责、禁止事项 | `docs/architecture.md` |
| 公开方法与上游接口契约 | `docs/endpoints.md` |
| 解析、分页、平台档案等长期规则 | `docs/parsing.md` |
| FFI / binding 契约 | `docs/ffi.md` |
| 测试与真实账号安全规则 | `docs/testing.md` |
| 尚未实现 / 尚未验证范围 | `docs/pending.md` |
| 发布流程、资产与校验 | `docs/RELEASING.md` |
| 精确发布工具版本 | `.github/workflows/release.yml` 与生成的 release manifest |
| 历史证据 | `docs/history/`（只读参考，不是现行真源） |

## Task Routing

- 改架构或模块边界 → 先读 `docs/architecture.md`
- 新增 / 修改接口 → `docs/endpoints.md` + `docs/parsing.md` + `docs/development.md`
- 改 `#[data]` / `#[export]` / `boltffi.toml` → `docs/ffi.md`
- 改测试、真机验证、账号写操作 → `docs/testing.md`
- 改发布、tag、资产、checksum → 只读 `docs/RELEASING.md`

## Non-negotiable Boundaries

- 同一个上游能力只允许一个生产实现；stdio 与 typed API 只能做适配。
- 对 QQ 上游的 HTTP 统一经过共享 `Upstream`；不得为新接口另造 HTTP / guard 栈。
- 不在活文档记录某个消费端“现在装了哪个版本、下一步如何升级”等状态。
- 不把本地参考目录、agent 文件或历史报告当开发依赖。
- 不创建第二份 release 指南，也不在 README / AGENTS 重复精确版本号。

## Quality Gates

默认提交前至少执行：

1. `cargo test --all-targets --locked`
2. `python3 tests/test_write_smoke.py`
3. `cargo clippy --all-targets --locked`
4. `bash scripts/check_repository_rules.sh`

改动 FFI 表面后，再按 `docs/ffi.md` 生成并检查对应 bindings。真实网络 / 真实账号测试均为显式手动门，
严格遵守 `docs/testing.md`；不得为了“验证接口”自行扩大写操作。

格式化策略与具体开发步骤以 `docs/development.md` 为准。
