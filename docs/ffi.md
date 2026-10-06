# 跨平台调用（BoltFFI）

同一份 Rust 内核通过 [BoltFFI](https://github.com/boltffi/boltffi) 生成各语言的绑定，
所以 macOS/iOS 应用与 Android 应用共用一套实现。

组件版本从 **0.1.0** 升为 **0.2.0**：BoltFFI 模型新增了曲目文件大小/曲风、专辑曲数、结构化 QRC（`QrcLine`/`QrcWord`）、
分页偏移（`TrackPage.nextOffset`、新 `AlbumPage` 的 `nextOffset`）与喜欢写回执（`LikeReceipt`），并增加了初始化和分页等导出。
JSON `PROTOCOL_VERSION` 仍为 **2**，旧的 stdio 请求与返回字段保持兼容；
FFI 的 Rust 数据模型按字段顺序编码，因此 Kotlin/Swift bindings 与 native library 必须由同一版本一起生成和打包，
不能把旧 bindings 与 0.2.0 native 混用。

## 生成与打包

```sh
cargo install boltffi_cli          # 一次性
boltffi init --name qqmusic_api_helper_next    # 已随仓库提交，通常不需要再跑

boltffi generate swift             # → dist/apple/Sources/QqmusicApiHelperNextBoltFFI.swift
boltffi generate kotlin            # → dist/android/kotlin/…/QqmusicApiHelperNext.kt + JNI 头
boltffi generate python            # → Python 绑定

boltffi pack apple                 # → xcframework + Package.swift
boltffi pack android               # → jniLibs
boltffi check                      # 检查工具链与 rustup target
```

配置在仓库根的 `boltffi.toml`（`boltffi init` 生成，Apple 部署目标、模块名、Kotlin 包名等都在里面）。
`dist/` 不进版本库。

## 生成的 API 形状

Rust 侧 `#[export] pub fn liked_songs(page: u32, limit: u32) -> Result<LikedSongs, HelperError>`
在两端分别长成：

```swift
// Swift：camelCase，错误变成 throws
public func likedSongs(page: UInt32, limit: UInt32) throws -> LikedSongs
public func importCredential(uin: String, qmKeyst: String) throws
public func loginStatus() throws -> LoginStatus
```

```kotlin
// Kotlin：同样的名字与类型
fun likedSongs(page: UInt, limit: UInt): LikedSongs
fun importCredential(uin: String, qmKeyst: String)
fun loginStatus(): LoginStatus
```

`#[data]` 的模型两端都是普通值类型（Swift `struct Track: Hashable, Equatable, Sendable`，
Kotlin data class），字段名与 Rust 一致（`songMid`、`albumId`、`imageURL`…）。

## 宿主需要做的两件事

### 1. 告诉组件数据目录

凭据落在宿主自己的可写目录里；组件不去猜平台路径。FFI 宿主在第一次调用任何 HelperNext API **之前**调用
`initialize(dataDir, platform)`。相同目录和档案可重复初始化；冲突配置或首次 API 调用之后才初始化会返回错误。
`platform` 使用 `web` 或 `android`。旧 Rust `configure(Configuration { .. })` 仍保留给 Rust 宿主。

```swift
try initialize(dataDir: appSupportURL.path, platform: "web") // macOS/iOS：Application Support
```

```kotlin
initialize(context.filesDir.absolutePath, "android") // Android：filesDir
```

### 2. 别在主线程调用

导出的函数是**同步阻塞**的（内部是一个 HTTP 往返，最长 20 秒超时）。这是有意的：BoltFFI 的 async
导出需要宿主提供运行时，而"在后台线程调用同步函数"在两个平台上都是一行的事。

```swift
Task.detached { let liked = try likedSongs(page: 1, limit: 50) ; await MainActor.run { … } }
```

```kotlin
viewModelScope.launch(Dispatchers.IO) { val liked = likedSongs(1u, 50u) }
```

> 也可以做成 `async` 导出（BoltFFI 支持，Swift 得到 `async throws`、Kotlin 得到 suspend + `FfiException`），
> 那需要给内核加一个小执行器；同步 + 后台分派已经够用，所以不做。

## 生成器的限制

BoltFFI 编译不过"多语句 wire writer"的形状：**元组向量** `Vec<(String, String)>` 会让
`generate swift` 直接失败（`swift target cannot render multi-statement codec write`），
`generate kotlin` 则跳过那个函数并打印

```
kind         name                 reason
function     import::cookies      multi-statement wire writer
```

**解决办法是把元组换成具名 `#[data]` 结构**，或者使用具名参数。旧的
`import_credential(uin, qm_keyst)` 保持不变；`import_credential_with_encrypt_uin(uin, qm_keyst, encryptUin)`
是接收可选加密 UIN 的新增导出。

Android pack 为 16 KB memory page 设置 ELF 最大 page size；release Cargo profile 会 strip 符号。
打包后仍应对四个 ABI 的最终 `.so` 检查 `LOAD` 段对齐和导出符号。

## 子进程方式（不需要 FFI 的宿主）

`src/bin/stdio.rs` 提供一个"一行一个 JSON"的适配器：请求 `{"id","method","params"}`，
响应回带同一个 `id`。适合不便做 FFI 的宿主，也适合命令行排查：

```sh
QQMUSIC_HELPER_NEXT_DIR=~/.local/share/helper-next ./target/release/qqmusic-helper-next
{"id":"1","method":"get_login_status","params":{}}
{"id":"1","ok":true,"login":{"loggedIn":true,…}}
```

适配器与 FFI 走的是**同一份实现**（`methods::dispatch`），不会出现两边行为不同的问题；
`api.rs` 的测试 `api_surface_matches` 保证方法表与类型化 API 一一对应。
