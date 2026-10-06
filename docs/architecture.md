# 架构：为什么这样分层

本文回答两个问题：一次调用如何流过组件（数据流），以及每个模块管什么、禁止管什么（职责边界）。
按固定清单跑过的一次性「边界体检」快照见
[history/repository-audit-2026-10-06.md](history/repository-audit-2026-10-06.md)，
长期护栏是 `scripts/check_repository_rules.sh`。改动方式见 [development.md](development.md)，
接口契约见 [endpoints.md](endpoints.md) 与 [parsing.md](parsing.md)。

## 1. 数据流

两个调用面，一条实现路径：

```text
macOS 宿主（子进程）              Android 宿主（如 NeuMusic）
   │ stdin/stdout 一行一个 JSON        │ BoltFFI 生成的 Kotlin 绑定
   ▼                                  ▼
src/bin/stdio.rs                    #[export] 类型化 API
（行协议适配器：                     （src/api.rs 的包装，以及
  id 关联、resultId、                  src/port/* 各领域自带的
  诊断走 stderr）                       #[export] 包装）
   │                                  │
   └────────────────┬─────────────────┘
                    ▼
        src/methods.rs —— 协议分发
        「方法名 → 端点实现」：既有表 → port 层 → 内建方法
        （get_helper_info 广告两者并集；版本与协议常量在这层）
                    │
      ┌─────────────┼──────────────────────┐
      ▼             ▼                      ▼
src/catalog.rs   src/port/*（移植层，      内建方法：
（既有端点：      一领域一文件，            限流/熔断配置、aria2_*、
 目录/榜单/       参考实现新接口）           登录状态、helper info
 歌词/取流/搜索）
      └─────────────┼──────────────────────┘
                    ▼
        src/upstream.rs —— 唯一的 QQ 上游 HTTP
        musicu.fcg · 签名 musics.fcg（zzc）· 老 fcgi
        平台档案 Platform · 限流/熔断（guard.rs）· 设备身份（device.rs）· 凭据（credential.rs）
                    │
                    ▼
            u.y.qq.com / c.y.qq.com
```

下载不走上面这条链：`src/aria2.rs` 把字节搬运交给本地的 `aria2-next` 进程，
经 **loopback** JSON-RPC（非默认端口 16800）通信。它不是「上游」，也不占用 QQ 上游的 agent。

## 2. 三个分层决策，以及为什么

**一个实现、两个调用面。** typed FFI 与 stdio 适配器都不是端点的第二份实现：stdio 的未知方法
一律交给 `methods::dispatch`（`src/bin/stdio.rs` 主循环），FFI 包装经由 `api.rs` 的
`call`/`port::call` 走同一个 `dispatch`。这样「CLI 能用、绑定里没有」这类漂移在结构上不可能发生，
`api_surface_matches` 测试（定义在 `src/api.rs`）再把方法表与类型化包装钉死一次。

**协议层是「方法名 → 端点」的表，不是业务逻辑。** `methods::dispatch`（`src/methods.rs`）
只做路由：先查既有表（`catalog_dispatch`，优先级最高），再试移植层（`port::dispatch`，因此移植的
新方法永远不会遮蔽既有方法），最后是内建方法。`get_helper_info` 广告的方法列表是两者并集，
宿主枚举到的就是全部能力。

**移植层独立成 `src/port/`，一领域一文件。** 参考实现（QQMusicApi）里尚未移植的接口持续落进
port 层，而既有代码是已上线契约、不能被顺手改动——把两者隔开，移植就永远不破坏存量。
一个领域文件自带 `METHODS`、`dispatch`、`#[data]` 模型、`#[export]` 包装和测试；
`src/port/mod.rs` 集中登记并自带两条确定性测试（每个方法必须有同名包装且在 `METHODS`
与 `dispatch` 两处出现；方法名跨领域不得重复）。签名类端点共用的 `zzc` 签名在
`src/port/signed.rs`（无独立导出，是领域文件的协议支撑）。

## 3. 模块职责与禁止事项

| 模块 | 职责 | 禁止 |
|---|---|---|
| `src/lib.rs` | 组合根：`initialize`/`configure`（首次生效后冻结，防止运行中挪走凭据目录）、`HelperError` 定义 | 不承载端点逻辑 |
| `src/api.rs` | 类型化公开 API（`#[export]`）；协议信封 → 模型的 `parse` 适配；共享 `Upstream` 的唯一发放点 | 不新增第二个 HTTP agent；不为省事复制端点实现 |
| `src/models.rs` | `#[data]` 数据模型，camelCase，一份定义同时服务 JSON 与 FFI | 字段不重排、不改名（FFI 按字段顺序编码） |
| `src/methods.rs` | 方法表 `METHODS`、`dispatch` 路由、内建方法、版本常量（数值以代码为准） | 不把端点业务写进路由层；协议版本不轻易变动 |
| `src/catalog.rs` | 既有目录/榜单/歌词/取流/搜索端点的实现与解析 | 既有形状是契约，改前先读 `docs/parsing.md` 的结论 |
| `src/port/*` | 参考实现新端点，一领域一文件，自带模型/包装/测试 | 不碰 `src/api.rs` 等既有文件；不遮蔽既有方法 |
| `src/port/signed.rs` | `musics.fcg` 的 `zzc` 签名（SHA-1 移植，照抄参考实现，签名对不上直接回 `2000`） | 不「发明等价算法」；签名必须作用于实际发送的字节 |
| `src/upstream.rs` | 唯一的 QQ 上游 HTTP；`Platform`（web/android）档案的 `comm` 叠加；挂载限流器、熔断器、设备身份、`encrypt_uin` 缓存 | 其他任何模块不得对 QQ 上游发 HTTP |
| `src/guard.rs` | 按内容类别分桶的限流（Read/Interactive/Playback/Account/Write，超限等待不丢弃）与熔断（阈值开路、半开探测） | 不在调用方各自实现节流 |
| `src/credential.rs` | 凭据读写（与被替换的 Python helper 同一文件，可互换）、`g_tk = hash33(qm_keyst)`、cookie 头组装 | 凭据不进日志、不进回包、不进文档 |
| `src/device.rs` | QIMEI 设备身份（RSA+AES 协议复刻），生成一次存盘，仅 android 档案使用 | 不重复申请设备身份；HTTP 复用传入的共享 agent |
| `src/aria2.rs` | 本地下载引擎托管：按需启动、JSON-RPC over loopback、宿主消失时随进程退出 | 不把 aria2 的 RPC 混入上游 agent；不常驻后台 |
| `src/login.rs` | QQ 扫码登录五步（`ptqrshow` → 轮询 → `check_sig` → authorize → QQLogin），流程无状态 | 不在组件里保存登录会话状态 |
| `src/qrc.rs` | QRC 逐字歌词：私有 DES 解密 → XML 取载荷 → 解析 → word-LRC；由已知答案向量钉住 | 解密算法照规范移植，不自创 |
| `src/bin/stdio.rs` | 行协议适配器：`id` 关联、资源 `id` → `resultId`、诊断走 stderr、父进程看门狗、退出时关停 aria2 | 不实现端点；stdout 只出协议 JSON，凭据不进任何一行 |

## 4. 横向约束（所有模块共同遵守）

1. **一套 Upstream、一套凭据、一套 guard。** 共享实例经 `api.rs` 的 `upstream()`（`OnceLock`）与
   `shared_upstream()` 发放；port 层包装统一走 `port::call`。再造一份就等于绕过限流与熔断。
2. **对 QQ 上游的 HTTP 只在 `src/upstream.rs`。** `device.rs` 复用传入的 agent；
   `aria2.rs` 的 HTTP 是本地 loopback RPC，与上游无关。
3. **凭据是单点职责。** 只有 `credential.rs` 读写字凭据；回包、日志、报告里不得出现凭据内容。
4. **`api_surface_matches` 是方法表与类型化 API 的同步闸。** 新增方法必须同时落 `METHODS`
   与 `#[export]`（或登记别名），测试失败即打回。
5. **平台档案是按接口的实测结论，不是全局开关。** 选错档案不报错、只回空数据或风控码；
   规则见 [parsing.md](parsing.md) 第一节。
