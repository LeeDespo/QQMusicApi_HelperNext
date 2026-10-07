<div align="center">
  <h1>QQMusicApi_HelperNext</h1>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/Rust-edition%202021-orange" alt="Rust"></a>
  <a href="https://github.com/boltffi/boltffi"><img src="https://img.shields.io/badge/bindings-BoltFFI-purple" alt="BoltFFI"></a>
  <a href="https://github.com/LeeDespo/QQMusicApi_HelperNext/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--or--later-green" alt="License"></a>
</div>

---

> [!IMPORTANT]
> **音乐平台不易，请尊重版权，支持正版。**

## 📖 介绍

**QQMusicApi_HelperNext 是一个面向 QQ 音乐互操作场景的 Rust 跨平台组件。**

仓库本身负责 QQ 音乐协议调用、请求签名、响应解析、凭据与设备身份、限流/熔断以及下载引擎托管，
并提供两个共享同一实现的调用面：

* **stdio 子进程**：一行一个 JSON，请求最终汇入统一的方法分发；
* **BoltFFI typed API**：从同一份 Rust 公开表面生成 Swift / Kotlin 等绑定。

项目的协议与接口认知部分来源于 [QQMusicApi](https://github.com/L-1124/QQMusicApi)；
当前架构、行为、测试与发布规则以**本仓库**为准，来源关系与许可证说明见 [NOTICE](NOTICE)。

## 🧭 正式交付范围

| 调用面 | 状态 |
|---|---|
| **stdio** `qqmusic-helper-next`（macOS ARM64） | ✅ GitHub Release 正式交付；协议行为有离线测试覆盖 |
| **Android FFI**（BoltFFI，四 ABI） | ✅ GitHub Release 正式交付；打包检查包含四 ABI 与 16 KB page-size |
| **Apple typed FFI** | ⚠️ 可生成绑定，但当前不作为正式 Release 资产 |
| **wasm** | ❌ 未适配、未验证，配置关闭 |
| **Windows / Linux 可执行交付** | ❌ 当前不发布 |

“能生成”不等于“正式支持”。正式 Release 的目标、资产名与校验规则只以
[docs/RELEASING.md](docs/RELEASING.md) 为准。

## 🚀 快速开始

### Rust

```rust
use qqmusic_api_helper_next::{api, configure, Configuration, Platform};

fn main() -> Result<(), qqmusic_api_helper_next::HelperError> {
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

### 作为子进程

```sh
echo '{"id":"1","method":"get_helper_info","params":{}}' | qqmusic-helper-next
# {"id":"1","ok":true,"helper":{"helperVersion":"<component-version>","protocolVersion":2,…}}
```

生成绑定与本地打包时，工具版本应与 Release workflow 的固定版本保持一致；具体命令与兼容规则见
[docs/ffi.md](docs/ffi.md)。`dist/` 是构建输出，不进版本库。

## ✨ 能力概要

**账号**：登录状态、网页 cookie 导入、凭据刷新、退出登录；扫码与手机验证码登录端点；
我喜欢、歌单、收藏专辑、关注歌手等账号资产与关系读取，以及受保护的可逆写操作。

**曲库**：搜索、歌单 / 排行榜 / 专辑 / 歌手 / MV / 电台 / 推荐、新歌与新碟；
歌词（整行 / 逐字 / 音译 / 翻译 / 助唱标注）；多档音质取流与批量取流；
评论读写；本地曲库封面匹配。

**运行时**：按内容类别分桶的限流与熔断、Aria2 下载引擎托管。

公开方法、上游 module / method、参数与回值见 [接口清单](docs/endpoints.md)；
分页、平台档案与字段映射等实现约束见 [解析要点](docs/parsing.md)。

## 📚 文档导航

* **[docs/README.md](docs/README.md)** —— 文档门户
* **[docs/endpoints.md](docs/endpoints.md)** —— 组件公开方法与上游接口契约
* **[docs/parsing.md](docs/parsing.md)** —— 长期解析与兼容规则
* **[docs/ffi.md](docs/ffi.md)** —— FFI 契约与绑定生成规则
* **[docs/testing.md](docs/testing.md)** —— 测试分层与真实账号安全规则
* **[docs/pending.md](docs/pending.md)** —— 组件自身尚未实现 / 尚未验证的范围
* **[docs/RELEASING.md](docs/RELEASING.md)** —— 发布规则唯一真源
* **[CHANGELOG.md](CHANGELOG.md)** —— 对外变化记录
* **[docs/history/](docs/history/)** —— 历史验证与审计记录

## ⚠️ 用途声明

本项目仅用于技术研究、学习、个人使用及互操作性验证。

项目作者不鼓励、亦不认可将本项目用于商业服务、批量数据获取、版权内容再分发或其他可能
侵犯腾讯、QQ 音乐及相关权利人权益的用途。请尊重版权并支持正版。

软件代码依据 **GPL-3.0-or-later** 提供；上述用途声明表达项目定位与作者立场，
**不构成额外许可证限制，也不改变 GPL 授予的权利**。使用者须自行确保行为符合服务条款、
版权规定及适用法律。

## 📄 许可证与第三方声明

* 软件许可证：**[GPL-3.0-or-later](LICENSE)**。
* 协议研究来源、非官方关系与权利边界见 [NOTICE](NOTICE)。
* Release 包携带依赖许可证汇总 `THIRD-PARTY-LICENSES.txt`，由发布脚本根据锁定依赖生成。

## 👥 致谢

* [QQMusicApi](https://github.com/L-1124/QQMusicApi) —— 协议与接口研究的重要来源
* [BoltFFI](https://github.com/boltffi/boltffi) —— 跨语言绑定生成
* [ureq](https://github.com/algesten/ureq) / [serde](https://github.com/serde-rs/serde) —— HTTP 与序列化
