<div align="center">
  <h1>QQMusicApi_HelperNext</h1>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/Rust-edition%202021-orange" alt="Rust"></a>
  <a href="https://github.com/L-1124/QQMusicApi"><img src="https://img.shields.io/badge/based%20on-QQMusicApi-blue" alt="Based on QQMusicApi"></a>
  <a href="https://github.com/boltffi/boltffi"><img src="https://img.shields.io/badge/bindings-BoltFFI-purple" alt="BoltFFI"></a>
  <a href="https://github.com/LeeDespo/QQMusicApi_HelperNext/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--or--later-green" alt="License"></a>
</div>

---

> [!IMPORTANT]
> **音乐平台不易，请尊重版权，支持正版。**

---

## 📖 介绍

**这是一个基于 [QQMusicApi](https://github.com/L-1124/QQMusicApi) 的 Rust 跨平台组件。**

QQMusicApi 用 Python 实现了 QQ 音乐接口的完整协议工作（请求签名、公共参数、各接口的 module/method/param、
响应模型的 jsonpath）。本项目把这套协议工作**移植到 Rust**，做成一个可以被多种宿主直接嵌入的组件：

* **一份 Rust 内核**：接口调用、请求签名、JSON 解析、凭据、限流、熔断、下载引擎托管都在这里；
* **两个调用面、一个实现**：宿主可以把组件当子进程驱动（一行一个 JSON 的 stdio 协议），
  也可以嵌入 [BoltFFI](https://github.com/boltffi/boltffi) 从同一份 `#[export]` 表面生成的
  Swift / Kotlin 绑定；两条路汇入同一个方法表，对上游的 HTTP 只存在于内核一处；
* **一份文档**：每个接口的上游形状、解析要点与验证边界都记录在 `docs/`，从下方导航进入。

它不解释"如何获取 QQ 音乐的数据"，那部分是 QQMusicApi 的文档；这里讲的是**组件如何被调用、如何实现、
如何跨平台**。

## 🧭 多端适配程度

| 调用面 | 现状 |
|---|---|
| **stdio 子进程** `qqmusic-helper-next`（macOS ARM64） | ✅ **正式**：一行一个 JSON 的子进程协议，macOS 宿主的当前消费方式，也是 Release 的 macOS 交付面；协议行为有测试覆盖 |
| **Android FFI**（BoltFFI，四 ABI） | ✅ **正式**：0.2.0 绑定与 JNI 库经 NeuMusic 集成并完成真账号只读与一次可逆写验证；四 ABI 编译与 16 KB 页对齐检查通过 |
| **macOS Swift typed FFI** | ⚠️ **在用、待成套替换**：现有安装仍是 0.1.0 产物；0.2.0 绑定已生成；类型检查与真实 FFI 程序验证记录在宿主侧接入报告（`Music_app/docs/helpernext-integration-2026-10-05.md`）。不随 Release 发布（`include_macos = false`） |
| **iOS / Apple XCFramework** | ❌ 绑定可生成，但无验证记录，不随 Release 发布 |
| **wasm** | ❌ 仅 `boltffi.toml` 生成配置（`wasm32-unknown-unknown`）：无工具链、无适配代码、无验证记录，不得宣称支持 |
| **Windows / Linux** | ❌ 未适配 |

「能生成」不等于「正式支持」：哪些平台随 Release 交付、资产如何成套打包，以
[docs/release.md](docs/release.md) 为准。

## 🚀 快速开始

### Rust

```rust
use qqmusic_api_helper_next::{api, configure, Configuration, Platform};

fn main() -> Result<(), qqmusic_api_helper_next::HelperError> {
    // 宿主自己的可写目录；组件不会去猜平台的路径。
    configure(Configuration {
        data_dir: "/path/to/app-support".into(),
        default_platform: Platform::Web,
    });

    if api::login_status()?.logged_in {
        let liked = api::liked_songs(1, 50)?;
        println!("{} 首 / 共 {} 首", liked.tracks.len(), liked.total);
    }
    Ok(())
}
```

### Swift（BoltFFI 生成）

```swift
import QqmusicApiHelperNext

try initialize(dataDir: appSupport.path, platform: "web")
if try loginStatus().loggedIn {
    let liked = try likedSongs(page: 1, limit: 50)
    print("\(liked.tracks.count) / \(liked.total)")
}
```

### Kotlin（BoltFFI 生成）

```kotlin
initialize(context.filesDir.absolutePath, "android")
if (loginStatus().loggedIn) {
    val liked = likedSongs(page = 1u, limit = 50u)
    println("${liked.tracks.size} / ${liked.total}")
}
```

### 作为子进程（一行一个 JSON）

```sh
echo '{"id":"1","method":"get_helper_info","params":{}}' | qqmusic-helper-next
# {"id":"1","ok":true,"helper":{"helperVersion":"0.2.0","protocolVersion":2,…}}
```

生成绑定与打包：

```sh
cargo install boltffi_cli
boltffi generate swift     # → dist/apple/Sources/*.swift
boltffi generate kotlin    # → dist/android/kotlin/…/*.kt
boltffi pack apple         # → dist/apple 的 xcframework / Package.swift
boltffi pack android       # → dist/android 的 jniLibs
```

`dist/` 不进版本库；bindings 与 native library 必须同版本成套生成、成套替换，规则见
[docs/ffi.md](docs/ffi.md)。

## ✨ 能力概要

**账号**：登录状态、网页 cookie 导入、凭据刷新、退出登录；扫码与手机验证码登录端点；
我喜欢、歌单、收藏专辑、关注的歌手等账号资产与关系读取，歌单与收藏类可逆写入
（`set_liked` / 数字 ID 回执 `set_liked_by_id`）。

**曲库**：搜索（全部 10 个类型）、歌单 / 排行榜 / 专辑 / 歌手 / MV / 电台 / 推荐、新歌与新碟上架；
歌词（整行 / **逐字毫秒** / 音译 / 翻译 / 助唱标注）；六档音质阶梯取流与批量取流；
评论（数量 / 热评 / 新评 / 发 / 删）；本地曲库封面匹配。

**运行时**：按内容类别分桶的限流与熔断（可配置）、Aria2 下载引擎托管（排队 / 进度 / 暂停 / 取消）。

每个方法背后的上游 module / method、参数与回值见[接口清单](docs/endpoints.md)；
分页语义、平台档案等不直观但必须照做的解析规则见[解析要点](docs/parsing.md)。

## 📚 文档导航

* **[AGENTS.md](AGENTS.md)** —— 仓库守则：任务路由、硬性规则、完成清单（开工先读）
* **[docs/architecture.md](docs/architecture.md)** —— 分层与模块职责、边界体检
* **[docs/development.md](docs/development.md)** —— 改动流程与新增接口检查表
* **[docs/endpoints.md](docs/endpoints.md)** —— 接口清单：每个方法的上游 module/method、参数与回值
* **[docs/parsing.md](docs/parsing.md)** —— 解析要点：平台档案、字段映射、设备身份等必读结论
* **[docs/ffi.md](docs/ffi.md)** —— BoltFFI 绑定生成、成套更新规则与调用示例
* **[docs/testing.md](docs/testing.md)** —— 测试分层、真机只读与凭据纪律
* **[docs/pending.md](docs/pending.md)** —— 待决清单与已验证边界
* **[docs/release.md](docs/release.md)** —— 发布规则（唯一真源）
* **[docs/history/](docs/history/)** —— 历史验证报告与取证记录

## 📄 许可证

本项目采用 **[GNU General Public License v3.0 or later](LICENSE)**，与 QQMusicApi 保持一致——
本项目是它的 Rust 移植，协议工作与接口认知来自该项目。

本项目仅用于对技术可行性的探索及研究，请勿将其用于任何商业用途或侵犯版权的行为。

## ⚠️ 免责声明

由于使用本项目产生的包括由于本协议或由于使用或无法使用本项目而引起的任何性质的任何直接、间接、特殊、
偶然或结果性损害（包括但不限于因商誉损失、停工、计算机故障或故障引起的损害赔偿，或任何及所有其他商业
损害或损失）由使用者负责。

## 👥 致谢

* [QQMusicApi](https://github.com/L-1124/QQMusicApi) —— 本项目的协议与接口来源
* [BoltFFI](https://github.com/boltffi/boltffi) —— 跨语言绑定生成
* [ureq](https://github.com/algesten/ureq) / [serde](https://github.com/serde-rs/serde) —— HTTP 与序列化
