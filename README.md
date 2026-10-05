<div align="center">
  <h1>QQMusicApi_HelperNext</h1>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/Rust-1.75%2B-orange" alt="Rust"></a>
  <a href="https://github.com/L-1124/QQMusicApi"><img src="https://img.shields.io/badge/based%20on-QQMusicApi-blue" alt="Based on QQMusicApi"></a>
  <a href="https://github.com/boltffi/boltffi"><img src="https://img.shields.io/badge/bindings-BoltFFI-purple" alt="BoltFFI"></a>
  <a href="https://github.com/LeeDespo/QQMusicApi_HelperNext/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--or--later-green" alt="License"></a>
</div>

---

> [!IMPORTANT]
> **音乐平台不易，请尊重版权，支持正版。**

---

## 📚 快速链接

* **[📖 接口清单](docs/endpoints.md)** —— 每个方法对应的上游 module/method、参数与回值
* **[🔍 解析要点](docs/parsing.md)** —— 平台档案、字段映射、大小写、设备身份等必读结论
* **[📌 待决清单](docs/pending.md)** —— 需要拍板的设计选择、参考里未移植的接口、环境限制
* **[✅ 接续验证报告](docs/continuation-verification-2026-10-04.md)** —— 功能补完、真实账号测试与写入复原记录
* **[🧩 跨平台调用](docs/ffi.md)** —— BoltFFI 生成 Swift / Kotlin 绑定的方式与调用示例

## 📖 介绍

**这是一个基于 [QQMusicApi](https://github.com/L-1124/QQMusicApi) 的 Rust 跨平台组件。**

QQMusicApi 用 Python 实现了 QQ 音乐接口的完整协议工作（请求签名、公共参数、各接口的 module/method/param、
响应模型的 jsonpath）。本项目把这套协议工作**移植到 Rust**，做成一个可以被多种宿主直接嵌入的组件：

* **一份 Rust 内核**：接口调用、请求签名、JSON 解析、凭据、限流、熔断都在这里；
* **多语言绑定**：用 [BoltFFI](https://github.com/boltffi/boltffi) 从同一份 `#[export]` 表面生成
  Swift / Kotlin / Java / C# / TypeScript / Python 绑定，**同一个内核同时服务 macOS/iOS 与 Android 应用**；
* **一个进程适配器**：`qqmusic-helper-next` 用「一行一个 JSON」的方式被宿主当作子进程驱动，
  适合不方便做 FFI 的宿主，也适合命令行排查。

它不解释"如何获取 QQ 音乐的数据"，那部分是 QQMusicApi 的文档；这里讲的是**组件如何被调用、如何实现、
如何跨平台**。

## ✨ 项目特色

* 🦀 **纯 Rust 内核，零解释器** —— 静态链接，产物约 2 MB（对比 Python 实现的数十 MB 运行时）
* 📱 **跨平台** —— BoltFFI 生成 Swift xcframework / Android jniLibs + Kotlin，一份内核两端复用
* 🔐 **请求签名与公共参数内置** —— `g_tk = hash33(qm_keyst)`、web 平台档案、cookie 授权，调用方不必关心
* 🧵 **线程安全** —— 共享 HTTP agent、限流器与熔断器，任意线程、并发调用都安全
* 🚦 **内置限流与熔断** —— 按内容类别分桶限流（超限等待而非丢弃），失败达阈值开路并半开探测
* 📝 **接口与解析全部记录在案** —— 每个接口的上游形状、字段映射与解析要点都在 `docs/`

## 🧭 支持的能力

**账号**：扫码登录（QQ 二维码 + 状态轮询；微信二维码取码与状态查询）、网页 cookie 导入、手机验证码发送与登录、
凭据刷新、退出登录、登录状态。

**曲库**：我喜欢、我的歌单、收藏专辑、关注的歌手、歌单/排行榜/专辑曲目、歌手（歌曲 / 专辑 / 资料 / 简介 / 列表 / 索引 /
相似歌手 / 主页 Tab / 名称图像 / 歌手 MV）、排行榜分组、电台（分组 / 无穷轮播批次）、新歌、猜你喜欢、推荐（首页信息流 / 雷达 / 推荐歌单）、
搜索（歌曲 / 歌手 / 专辑 / 歌单 / 热搜 / 联想补全 / 快速搜索 / 综合搜索 / 全部 10 个搜索类型）、
新碟上架、指定用户喜欢的歌曲、歌曲与专辑简介、歌词（整行 / **逐字毫秒** / 音译、助唱标注、多风格翻译与 AI 词典）、取流地址（六档音质阶梯探测 / 批量取流 / CDN 调度）、
歌曲资产（批量信息 / 其他版本 / 制作人 / 收藏数）、歌曲关联（相似 / 标签 / 相关歌单 / 相关 MV / 乐谱）、
MV（详情 / 播放地址 / 分类列表）、评论（数量 / 热评 / 新评 / 推荐评 / 时刻评论 / 发 / 删）、
账号资产（收藏歌单 / 专辑 / MV、音乐基因、不喜欢列表）、账号关系（主页 / VIP / 关注的歌手 / 粉丝 / 好友 / 关注的人 /
创建的歌单）、集合写入（建 / 删歌单、歌单加删歌、收藏与取消收藏专辑）、
收藏与取消收藏（`set_liked` / 数字 ID 回执 `set_liked_by_id`）、本地曲库的封面匹配（歌曲 / 歌手 / 专辑）。
列表页统一按上游原始行数分页：喜欢 / 歌单 / 专辑 / 歌手曲目页回 `total` 与 `nextOffset`（含被过滤的行），歌手专辑页回 `total` 与每张曲数。

**运行时**：限流与熔断（可配置）、下载引擎的托管（Aria2 Next 的排队、进度、暂停、取消）。

每个方法背后的上游 module / method、参数与回值见[接口清单](docs/endpoints.md)；
那些不直观、但必须照做的解析规则见[解析要点](docs/parsing.md)；
需要拍板的设计选择与未移植接口见[待决清单](docs/pending.md)。

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

## 🗂 项目结构

```
src/lib.rs         组合根：配置、错误类型、模块装配
src/api.rs         #[export] 的公开 API（宿主与绑定都调这一层）
src/models.rs      #[data] 的数据模型（Track/Album/Artist/Playlist/…）
src/upstream.rs    上游两个客户端（musicu.fcg 与老 fcgi）、g_tk、字段取值
src/credential.rs  凭据读写、cookie 导入、hash33
src/guard.rs       限流与熔断
src/methods.rs     协议层：方法名 → 端点实现（子进程适配器与 API 共用）
src/bin/stdio.rs   子进程适配器
docs/              接口清单、解析结论、待决清单、跨平台调用
```

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
