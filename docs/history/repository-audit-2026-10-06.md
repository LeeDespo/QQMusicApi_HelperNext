# 仓库边界体检（一次性快照，2026-10-06）

这是 2026-10-06 按固定十项清单对仓库架构边界跑的一次性体检快照。其中的文件行号、
计数与文件清单只对当时的提交有效，作为历史证据保留，**不作为现行规范引用**。

长期执行的护栏是 `scripts/check_repository_rules.sh`；架构的现行描述见
[architecture.md](../architecture.md)。

体检清单固定十项，纯检查、不改代码。其中第 1、8、10 项在 qrc 向量与冒烟取证 JSON
迁移落地后的工作区复跑：

| # | 检查项 | 命令 / 依据 | 结果 |
|---|---|---|---|
| 1 | `api_surface_matches` 存在且可执行 | `cargo test` 全绿；测试在 `src/api.rs:555`（声明见 `src/api.rs:9-10`） | ✅ |
| 2 | Upstream 只有一套 | `src/upstream.rs` 单文件；共享实例 `src/api.rs:19-33`；port 层经 `port::call`（`src/port/mod.rs:100-109`） | ✅ |
| 3 | 凭据只有一套 | `src/credential.rs` 单文件（`for_directory`/`load`/`store`/`clear`） | ✅ |
| 4 | guard 只有一套 | `src/guard.rs` 单文件；限流器与熔断器挂在共享 `Upstream` 上（`src/upstream.rs:91-92`） | ✅ |
| 5 | 上游 HTTP 只在 `upstream.rs` | `grep -rn ureq src/`：命中 `upstream.rs`（agent）、`device.rs:205,347`（复用传入 agent）、`aria2.rs:494`（本地 loopback）、`login.rs`/`port/login_extra.rs`（仅错误类型匹配） | ✅ |
| 6 | port/ 一领域一文件、三处登记 | 12 个领域文件 + `signed.rs`（zzc 签名支撑，无 `METHODS`）+ `mod.rs`；登记点 `src/port/mod.rs:20-32`（mod）、`:39`（all_methods）、`:66`（dispatch）；确定性测试 `:172-196` | ✅ |
| 7 | 两个调用面一个实现 | `src/bin/stdio.rs:224` 与 `src/api.rs:49` 都汇入 `methods::dispatch`；`get_helper_info` 广告并集 | ✅ |
| 8 | `src/` 无临时/数据文件 | `find src -name "*_tmp.*" -o -name "*.hex" -o -name "*.json"` | ✅ 无命中。QRC 已知答案向量已迁移至 `tests/fixtures/qrc/qrc-vector.hex`，`src/lib.rs:237`、`src/catalog.rs:1601`、`src/port/library_extra.rs:555` 三处 `include_str!` 已同步改路径，迁移后 `cargo test` 复跑三个向量测试仍绿（直接删除会破坏它们） |
| 9 | 仓库卫生 | `git ls-files` 无 `dist/`、`target/`、`.zcodeignore` 条目；release 真源唯一：`docs/RELEASING.md` 存在、根目录无 `release_plan.md`、无 `docs/release.md` | ✅ |
| 10 | 无凭据泄漏 | 对 tracked 及待入库文本文件宽松扫描 `qm_keyst|musickey|qimei16|qimei36|encrypt_uin` 后跟引号值：0 命中；7 个冒烟取证 JSON（`docs/history/evidence/2026-10-04/`）逐个 `grep -c` 同样为 0 | ✅ |

未列入本体检的：`cargo fmt --check`。按仓库政策**任何人不得全仓跑 `cargo fmt`**，
只允许对改动文件执行 `rustfmt --edition 2021 <文件>`（见 [development.md](development.md)），
因此体检不含全仓格式断言。
