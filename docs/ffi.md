# 跨平台调用（BoltFFI）

同一份 Rust 内核通过 [BoltFFI](https://github.com/boltffi/boltffi) 生成语言绑定。
FFI 与 stdio 是两个调用面，不是两套业务实现。

组件版本的唯一真源是 `Cargo.toml package.version`；协议版本由生产代码定义。
任何一次 FFI 发布都必须把 **bindings + native library** 视为一个不可拆分的版本单元：
不得跨 Release 混用生成代码与 native library。具体历史 breaking changes 只记在
[CHANGELOG.md](../CHANGELOG.md) 与对应 release notes，不在本长期文档重复版本迁移史。

## 生成与打包

本地生成时使用与 `.github/workflows/release.yml` 固定值一致的 BoltFFI CLI，
不要直接依赖“当前 latest”：

```sh
cargo install boltffi_cli --version <release-workflow-version> --locked
boltffi generate swift
boltffi generate kotlin
boltffi generate python
boltffi pack apple
boltffi pack android
boltffi check
```

配置在根目录 `boltffi.toml`。生成输出位于 `dist/`，不进版本库。
哪些目标属于正式 Release，以 [RELEASING.md](RELEASING.md) 为唯一真源。

## 生成的 API 形状

Rust 侧的 `#[export]` 函数生成 Swift / Kotlin 的类型化 API。例如：

```swift
public func likedSongs(page: UInt32, limit: UInt32) throws -> LikedSongs
public func importCredential(uin: String, qmKeyst: String) throws
public func loginStatus() throws -> LoginStatus
```

```kotlin
fun likedSongs(page: UInt, limit: UInt): LikedSongs
fun importCredential(uin: String, qmKeyst: String)
fun loginStatus(): LoginStatus
```

`#[data]` 模型是公开 wire / ABI 契约的一部分。字段调整必须按兼容性评估处理，并在同一次
Release 中重新生成所有正式交付目标的 bindings 与 native library。

## 初始化契约

FFI 调用方必须在第一次调用其他 HelperNext API 之前调用 `initialize(dataDir, platform)`：

- `dataDir`：调用方提供的可写目录；组件不猜平台路径；
- `platform`：当前支持 `web` 或 `android`；
- 同一配置可重复初始化；冲突配置或过晚初始化应返回错误。

Rust 直接使用时仍可调用 `configure(Configuration { .. })`。

## 同步调用

当前导出函数是同步阻塞调用。调用方应在自己的后台执行上下文中调用，避免阻塞 UI / 主线程。
组件不为某个具体 App 绑定线程模型；需要 async 表面时，应作为组件级 API 设计单独评审。

## 生成器限制

BoltFFI 对部分复合 wire writer 形状有限制。例如元组向量可能导致目标语言生成失败或跳过导出。
遇到此类情况优先改为具名 `#[data]` 结构或具名参数，不在不同平台分别维护手写兼容层。

Android 打包需要检查正式四 ABI 的最终 `.so`、导出符号和 16 KB page-size 对齐。
精确工具链版本与发布检查由 Release workflow / manifest 记录。

## stdio 调用面

`src/bin/stdio.rs` 提供“一行一个 JSON”的协议适配器：

```json
{"id":"1","method":"get_login_status","params":{}}
```

响应回带同一个 `id`。stdio 与 typed API 必须汇入同一生产实现；
公开方法与字段语义以 [endpoints.md](endpoints.md) / [parsing.md](parsing.md) 为准。
