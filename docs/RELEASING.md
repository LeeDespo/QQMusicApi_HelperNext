# 发布规则（Release）

> **本文件是本仓库发布事务的唯一真源。** 不在根目录或其他位置维护 `release_plan.md` 之类的第二真源；
> 修改发布语义只改这里，并在 AGENTS.md / README 的发布路由保持指向本文件。
>
> 适用仓库：`LeeDespo/QQMusicApi_HelperNext`
> 组件版本基线：**0.2.0**（`Cargo.toml` 与 `src/methods.rs` 的 `COMPONENT_VERSION`）
> BoltFFI：**0.31.0**
> 当前发布范围：**macOS ARM64 stdio 子进程 + Android BoltFFI**
> 暂不发布：Windows、Linux、Apple FFI（macOS/iOS XCFramework）、**wasm**（见 §2.2）
>
> 语气约定：本文所有「必须 / 禁止 / 只」都是现行有效规则；§10 描述的 release CI
> 已由 `.github/workflows/release.yml` 与 `scripts/release/` 落地（见 §10、§17）。

---

## 1. 目标

GitHub Release 是组件的正式交付面。消费端（macOS 播放器、NeuMusic 等）**不依赖**：

- 本地 clone HelperNext 源码；
- dirty working tree；
- `source.patch`；
- 本地安装 Rust / Android NDK / BoltFFI 后自行构建；
- 手工复制某一个 `.so` 或某一份 Kotlin binding；
- 无法追溯来源的裸二进制。

正式 Release 必须同时满足：

1. 所有发布物来自同一个 Git tag；
2. 构建源代码是 clean tree；
3. Android Kotlin binding 与四 ABI native library 同次生成、成套发布；
4. 每个 Release 资产可通过 SHA-256 验证；
5. Release 能明确追溯到 Git commit、组件版本、协议版本和构建工具链；
6. 消费端只下载固定 Release 资产，不重新构建 HelperNext。

---

## 2. 当前发布范围

### 2.1 发布的两条消费链

```text
macOS ARM64
App
 ↓ stdin/stdout JSON lines
qqmusic-helper-next
 ↓
HelperNext Rust Core
```

```text
Android App
 ↓
BoltFFI generated Kotlin
 ↓ JNI
libqqmusic_api_helper_next.so
 ↓
HelperNext Rust Core
```

### 2.2 暂不发布清单

以下内容**不进入** Release：

```text
Windows executable / Windows FFI
Linux executable / Linux FFI
macOS FFI / XCFramework
iOS FFI / XCFramework
wasm 产物
Rust .rlib
裸 staticlib .a
单独散装的 Android .so
```

原因与现状（均已核实）：

- Windows / Linux 尚未完成适配与验证，不构建、不上传、不宣称支持（§13）；
- `boltffi.toml` 的 `[targets.apple]` 配置为 `include_macos = false`，macOS 消费方式以
  `qqmusic-helper-next` stdio 子进程为主（§13.1）；
- **wasm**：`boltffi.toml` 的 `[targets.wasm]` 已关闭（`enabled = false`，2026-10-07 起）——
  本机未安装 `wasm32-unknown-unknown` 工具链、`src/` 没有任何 wasm 相关代码、也没有任何验证记录。
  wasm 处于「未适配、未验证」状态，**任何人不得在 README、Release Notes 或其他文档中
  宣称支持 wasm**；启动适配时重新打开配置，并在本节回填工具链与验证方式；
- 未经过真实验证的平台不出现在正式 Release 中。以后新增平台时单独扩展本文的范围章节，不提前
  生成「看起来支持」的产物。

**CI build host ≠ 正式支持 target**：Linux runner 可以跑 Rust 测试和 Android 构建，
这不等于发布 Linux 可执行文件；同理，能 `boltffi generate` 出某一平台绑定不等于该平台被正式支持。

---

## 3. Release 资产清单

Release 页面只出现以下资产（以 v0.2.0 为例）：

```text
qqmusic-helper-next-v0.2.0-macos-arm64.tar.gz
qqmusic-helper-next-v0.2.0-android.zip
release-manifest.json
SHA256SUMS
THIRD-PARTY-LICENSES.txt
```

以及 GitHub 自动提供的 `Source code (zip / tar.gz)`（与 tag 一一对应）。

- `macos-arm64.tar.gz`：供直接启动 HelperNext 子进程的 macOS 消费端使用；
- `android.zip`：供 NeuMusic 等 Android 宿主使用；
- `release-manifest.json`：机器可读的 Release 元数据；
- `SHA256SUMS`：所有 Release 资产的哈希，CI 生成、不手填；
- `THIRD-PARTY-LICENSES.txt`：第三方依赖许可证汇总。

---

## 4. macOS 包

### 4.1 文件名

统一 `qqmusic-helper-next-v${VERSION}-macos-arm64.tar.gz`（如 `qqmusic-helper-next-v0.2.0-macos-arm64.tar.gz`）。
禁止模糊命名：`helper.zip`、`release.zip`、`mac-build.tar.gz`、裸的 `qqmusic-helper-next`。

### 4.2 包内容

```text
qqmusic-helper-next-v0.2.0-macos-arm64/
├── qqmusic-helper-next      # 唯一正式 executable（src/bin/stdio.rs）
├── LICENSE
├── NOTICE
├── THIRD-PARTY-LICENSES.txt
└── manifest.json
```

规则：

- 当前仓库只有一个 binary target（`Cargo.toml` 的 `[[bin]] qqmusic-helper-next`），
  包内只含这一个可执行文件；
- **任何文档或注释不得描述不存在的 binary。** `Cargo.toml` 头部对该残留的
  `qqmusic-helper-next-cli` 描述已在 0.2.0 发布前删除；在以本条为准的前提下，
  不发布、不宣称 cli；
- 不发布 `.rlib` 或裸 `.a` 当作通用二进制。

### 4.3 manifest.json

```json
{
  "name": "QQMusicApi_HelperNext",
  "componentVersion": "0.2.0",
  "protocolVersion": 2,
  "gitCommit": "<FULL_GIT_SHA>",
  "target": "aarch64-apple-darwin",
  "binary": "qqmusic-helper-next",
  "rustc": "<RUSTC_VERSION>",
  "sha256": "<BINARY_SHA256>"
}
```

目的：只拿到解压后离线目录的用户也能知道版本、来源 commit、target 与完整性。

---

## 5. Android 包

### 5.1 原子性原则

Android FFI 资产作为一个**不可拆分的原子包**发布。BoltFFI 的模型与 wire layout 按字段顺序编码、
可能随版本变化，因此：

> Kotlin binding + JNI 生成物 + 四 ABI `.so` 必须来自同一个 commit、同一次 BoltFFI 构建。

禁止消费端自行组合旧 Kotlin + 新 `.so`（或任何形式的混搭）；禁止只替换一个 ABI、
只替换 Kotlin binding。这与 [ffi.md](ffi.md) 的成套更新规则是同一条纪律。

Kotlin 包名 `com.example.qqmusic_api_helper_next` 是既有消费契约（`boltffi.toml` 的
`[targets.android.kotlin].package`），改名即破坏 NeuMusic。它作为**已知瑕疵**保留，暂不修改；
若未来确要改，按契约变更走 [pending.md](pending.md) 的评审流程并成套发布。

### 5.2 文件名

统一 `qqmusic-helper-next-v${VERSION}-android.zip`。

### 5.3 包结构

```text
qqmusic-helper-next-v0.2.0-android/
├── kotlin/
│   ├── com/example/qqmusic_api_helper_next/QqmusicApiHelperNext.kt
│   └── jni/
│       ├── jni_glue.c
│       └── qqmusic_api_helper_next.h
├── jniLibs/
│   ├── arm64-v8a/libqqmusic_api_helper_next.so
│   ├── armeabi-v7a/libqqmusic_api_helper_next.so
│   ├── x86/libqqmusic_api_helper_next.so
│   └── x86_64/libqqmusic_api_helper_next.so
├── LICENSE
├── NOTICE
├── THIRD-PARTY-LICENSES.txt
└── manifest.json
```

正式支持四个 ABI：`arm64-v8a`、`armeabi-v7a`、`x86`、`x86_64`。任何一个正式 ABI 构建失败，
**整个 Android Release 失败**，不发布残缺包。

---

## 6. Android manifest.json

```json
{
  "name": "QQMusicApi_HelperNext",
  "componentVersion": "0.2.0",
  "protocolVersion": 2,
  "gitCommit": "<FULL_GIT_SHA>",
  "boltffi": "0.31.0",
  "rustc": "<RUSTC_VERSION>",
  "androidNdk": "<NDK_VERSION>",
  "minSdk": 24,
  "abis": ["arm64-v8a", "armeabi-v7a", "x86", "x86_64"],
  "files": {
    "kotlin/com/example/qqmusic_api_helper_next/QqmusicApiHelperNext.kt": "<SHA256>",
    "kotlin/jni/jni_glue.c": "<SHA256>",
    "kotlin/jni/qqmusic_api_helper_next.h": "<SHA256>",
    "jniLibs/arm64-v8a/libqqmusic_api_helper_next.so": "<SHA256>",
    "jniLibs/armeabi-v7a/libqqmusic_api_helper_next.so": "<SHA256>",
    "jniLibs/x86/libqqmusic_api_helper_next.so": "<SHA256>",
    "jniLibs/x86_64/libqqmusic_api_helper_next.so": "<SHA256>"
  }
}
```

- `minSdk` 必须与 `boltffi.toml` 的 `[targets.android] min_sdk`（现为 24）一致；
- 正式 Release 不提供 `workingTreeChanges` 字段——正式构建的硬性要求是 clean tree（§9.1），
  dirty tree 在 CI 直接失败。

---

## 7. 顶层 release-manifest.json

描述整个 Release 而非单个包：

```json
{
  "name": "QQMusicApi_HelperNext",
  "version": "0.2.0",
  "tag": "v0.2.0",
  "protocolVersion": 2,
  "gitCommit": "<FULL_GIT_SHA>",
  "sourceTreeClean": true,
  "toolchain": {
    "rustc": "<RUSTC_VERSION>",
    "cargo": "<CARGO_VERSION>",
    "boltffi": "0.31.0",
    "androidNdk": "<NDK_VERSION>"
  },
  "artifacts": {
    "qqmusic-helper-next-v0.2.0-macos-arm64.tar.gz": {
      "kind": "stdio-binary",
      "target": "aarch64-apple-darwin",
      "sha256": "<SHA256>"
    },
    "qqmusic-helper-next-v0.2.0-android.zip": {
      "kind": "boltffi-android",
      "abis": ["arm64-v8a", "armeabi-v7a", "x86", "x86_64"],
      "sha256": "<SHA256>"
    }
  }
}
```

---

## 8. SHA256SUMS

Release 顶层必须提供 `SHA256SUMS`，覆盖全部资产：

```text
<sha>  qqmusic-helper-next-v0.2.0-macos-arm64.tar.gz
<sha>  qqmusic-helper-next-v0.2.0-android.zip
<sha>  release-manifest.json
<sha>  THIRD-PARTY-LICENSES.txt
```

CI 中生成，不手填。消费端校验：

```bash
sha256sum -c SHA256SUMS          # Linux
shasum -a 256 <file>             # macOS 自带
```

---

## 9. Git tag 与源码规则

### 9.1 三位一体

```text
Git tag = Cargo.toml version = Release version
```

例：`Cargo.toml` `version = "0.2.0"` ↔ tag `v0.2.0` ↔ Release `v0.2.0`。不一致即失败。

### 9.2 禁止 dirty tree 发布

正式构建前必须通过：

```bash
git diff --exit-code
git diff --cached --exit-code
test -z "$(git status --porcelain)"
```

任一失败即停止 Release。禁止「base revision + `source.patch`」作为正式发布方式
（消费端契约见 §12）。

### 9.3 Release 只能从 tag 构建

```text
push tag vX.Y.Z → GitHub Actions → checkout tag → test → build → package → checksum → release
```

本地可以构建测试，但正式 Release 产物只认 CI；禁止本地编译后手工上传正式 artifact。

---

## 10. Release CI（已实现）

**实施状态：已落地。** `.github/workflows/release.yml` 按本节结构实现；打包逻辑在
`scripts/release/`（`pack-macos.sh`、`pack-android.sh`、`assemble-release.sh`、
`third-party-licenses.sh`、`check_16kb_pages.py`），本地可用 `ALLOW_DIRTY_TREE=1`
以同一批脚本干跑演练。

本节原先要求的两个前置项，实际做法回填如下：

1. **stdio `--version`**：`src/bin/stdio.rs` 已支持 `--version`——在读取或创建任何宿主配置
   之前打印组件与协议版本并退出 0（`tests/stdio_version.rs` 钉住该契约）；macOS job 与包内 smoke
   都用它，不再需要 `get_helper_info` 解析方案；
2. **真实 tag 与 secret 环境**：release job 用 `GITHUB_TOKEN`（workflow `permissions: contents: write`）
   创建 Release，不需要额外 secret。

### 10.1 Trigger

只响应 `on: push: tags: ["v*"]`。不自动响应 branch push。

### 10.2 Job 结构

```text
validate
   │
   ├────────────┐
   ▼            ▼
macos       android
   │            │
   └──────┬─────┘
          ▼
       release
```

### 10.3 validate job

1. 校验 tag = `Cargo.toml` version（§9.1）；
2. `cargo test --all-targets`（全离线集合；真实账号 smoke 是独立人工/受控流程，
   **不得在公开 CI 运行凭据**，L4 规则见 [testing.md](testing.md)）；
3. `boltffi check --android`（Apple target 在 Linux runner 上无法检查，由 macOS job 的真实构建
   与 smoke 覆盖），加上仓库自身的 API surface 检查（`api_surface_matches` 已含在测试里）。

### 10.4 macOS build job

- Runner `macos`，只构建 `aarch64-apple-darwin`；
- `cargo build --release --bin qqmusic-helper-next`，`file` 确认产物是
  `Mach-O 64-bit executable arm64`；
- 最小 smoke：`--version`（做法见 §10 前置项回填），以及 stdin 发送
  `{"id":"1","method":"get_helper_info","params":{}}`，必须回
  `ok = true`、`helperVersion` = 当前版本、`protocolVersion` = 2。通过后才允许打包。

### 10.5 Android build job

- Runner `ubuntu-latest`；固定 Rust toolchain、Android NDK、BoltFFI 0.31.0，禁止 `latest` 漂移；
- 构建：`boltffi pack android --release --deny-skipped`。正式 Release 必须带 `--deny-skipped`——
  不允许某个 API 生成失败时只打印警告仍继续发布；
- **四 ABI 完整性**：`dist/android/jniLibs/{arm64-v8a,armeabi-v7a,x86,x86_64}/
  libqqmusic_api_helper_next.so` 四个都存在，缺一即 Release 失败；
- **16 KB page-size**：`boltffi.toml` 已配置 `-Wl,-z,max-page-size=16384`；CI 保留 ELF `LOAD`
  段对齐检查，不以「编译成功」作为 Android native 产物验收标准；
- **同源检查**：Kotlin binding 与四个 `.so` 在**同一 job** 生成并统一计算 SHA256、同 job 生成
  manifest；禁止从不同 CI run 拼装 Android zip。

### 10.6 release job

仅当 validate / macos / android 全部成功后运行：下载 artifact → 组装两个正式压缩包 →
生成 `release-manifest.json` → 生成 `SHA256SUMS` → 生成/复制 `THIRD-PARTY-LICENSES.txt` →
创建 GitHub Release 并上传资产。

---

## 11. GPL 与许可证

- 本仓库许可证为 **GPL-3.0-or-later**（与 QQMusicApi 一致：本项目是它的 Rust 移植）；
- 每个二进制包内必须携带 `LICENSE`、`NOTICE` 与 `THIRD-PARTY-LICENSES.txt`；
- `THIRD-PARTY-LICENSES.txt` 由 `scripts/release/third-party-licenses.sh` 从
  `cargo metadata --locked` 自动生成（标识符级汇总：依赖名、版本、声明的许可证、仓库地址；
  不内嵌各许可证全文），不长期手工维护；
- Release 与 Git tag 一一对应，GitHub 自动提供的 source archives 与二进制天然同源。

---

## 12. 消费端接入契约（NeuMusic 更新方式）

本仓库对外承诺的只有三样：**固定资产名 + release-manifest.json + SHA256SUMS**。
职责边界：**本仓库负责构建并发布组件；宿主负责下载、校验、消费**——宿主不再负责
重新构建 HelperNext，也不需要安装 Rust 或 BoltFFI。这是两个仓库最重要的职责分离。

NeuMusic（及其他宿主）的更新方式固定为：

```text
读取 helpernext.lock.json（锁定用的 Release 版本/资产/SHA）
        ↓
下载 android.zip 与 release-manifest.json / SHA256SUMS
        ↓
校验 SHA256
        ↓
解压到 staging
        ↓
检查 manifest（版本、commit、文件哈希）
        ↓
原子替换 app/helpernext/
        ↓
Gradle test/build
```

- 宿主侧维护 `helpernext.lock.json`，让「这个版本的 App 用哪个 HelperNext Release」
  成为 git 历史里可回答的问题；
- 更新脚本只做上面这条链，**不再** fetch HelperNext 源码、checkout revision、apply
  `source.patch`、安装/调用 boltffi、编译 Rust；
- `source.patch` 允许存在于本地实验、临时验证与未提交开发；**禁止**正式 master 依赖
  dirty HelperNext patch 重建正式组件。正式路径只有一条：
  HelperNext 修改 → commit → tag → Release → 宿主更新 lock。

---

## 13. 其他平台的现行处理

### 13.1 Apple FFI

`boltffi.toml` `[targets.apple]` 为 `include_macos = false`：本阶段 Release 不发布
XCFramework / Swift Package / macOS FFI / iOS FFI。macOS 的正式消费方式是
`qqmusic-helper-next` stdio 可执行文件。iOS 的 Swift 绑定与库**可生成但无验证记录**，
与「正式支持」是两回事。未来需要 Swift 直接 FFI 时，单独设计
`qqmusic-helper-next-vX.Y.Z-apple.zip`，并先完成 macOS/iOS target、XCFramework、
Swift binding、ABI 与真实宿主的验证；验证完成前不进 Release。

### 13.2 wasm

见 §2.2：`boltffi.toml` 配置已关闭（`enabled = false`），无工具链、无代码、无验证。
不构建、不上传、不宣称支持。适配工作启动时重新打开配置，并在本节回填工具链与验证方式。

### 13.3 Windows / Linux

不构建、不上传、不在 README Release Matrix 宣称支持。CI 也不增加 windows release 构建或
Linux executable 发布；Linux runner 只用于测试与 Android 构建。

---

## 14. Release Notes 模板

每次 Release 使用统一结构：

```markdown
# QQMusicApi_HelperNext v0.2.0

## Compatibility

- Component: 0.2.0
- JSON protocol: 2
- BoltFFI: 0.31.0

## Supported release targets

- macOS ARM64 — stdio binary
- Android — BoltFFI, arm64-v8a / armeabi-v7a / x86 / x86_64

Windows, Linux, Apple FFI and wasm are not currently released or supported.

## Breaking / binding changes

- ...

## Added

- ...

## Fixed

- ...

## Artifacts

- qqmusic-helper-next-v0.2.0-macos-arm64.tar.gz
- qqmusic-helper-next-v0.2.0-android.zip

Verify all assets with `SHA256SUMS`.
```

每次 Release 的正文放在 `docs/release-notes/vX.Y.Z.md`（随 tag 一起进仓库、可评审），
release job 用 `gh release create --notes-file` 直接读取；文件缺失即该次发布失败。

---

## 15. 禁止事项

```text
从 dirty working tree 发布
手工上传本地 build 的正式 artifact
Release tag 与 Cargo version 不一致
Android Kotlin 与 .so 分开更新
只替换一个 ABI
只替换 Kotlin binding
消费端自己维护 HelperNext raw protocol 副本
NeuMusic 正式版本依赖 source.patch
发布未经验证的 Windows / Linux binary
当前阶段发布未经验证的 Apple FFI / XCFramework
宣称支持 wasm 或发布 wasm 产物
将 .rlib 当成通用 binary artifact
把 release 真源挪出本文档（如在根目录新建 release_plan.md）
```

---

## 16. 验收清单

发布前逐项核对；这也是 release CI 各 job 的断言来源。

**版本与源码**

- [ ] `Cargo.toml` version 与 Git tag 一致
- [ ] Release 从 Git tag 自动触发，commit 可追溯
- [ ] 构建源树 clean；正式流程不存在 dirty source patch

**Rust / API**

- [ ] `cargo test --all-targets` 通过
- [ ] BoltFFI surface 检查通过；协议版本与组件版本正确

**macOS**

- [ ] 只发布 ARM64；二进制为 Mach-O arm64
- [ ] 版本 smoke（`--version` 或 `get_helper_info` 方案）通过
- [ ] archive 含 LICENSE 与 manifest

**Android**

- [ ] BoltFFI 版本固定 0.31.0；`--deny-skipped` 生效
- [ ] Kotlin binding 生成成功；四 ABI `.so` 全部存在且与 Kotlin 同一次构建
- [ ] 16 KB page-size 检查通过；manifest 完整、逐文件 SHA256 完整

**Release 资产**

- [ ] macOS / Android 压缩包、`release-manifest.json`、`SHA256SUMS`、`THIRD-PARTY-LICENSES.txt`
      全部上传
- [ ] GitHub source archives 与 tag 对应

**不支持的平台**

- [ ] 未上传 Windows / Linux 可执行文件，README 未宣称支持
- [ ] 未上传未经验证的 Apple FFI / XCFramework 与 wasm 产物

**消费端（NeuMusic）**

- [ ] `helpernext.lock.json` 锁定 Release 版本；更新脚本下载 Release 资产并校验 SHA256
- [ ] staging + 原子替换；更新不需要安装 Rust 或 BoltFFI
- [ ] 正式流程不依赖 `source.patch`

---

## 17. 实施状态

规则已生效，以下事项**尚未落地**，完成时回填本节：

| 事项 | 状态 | 依赖 |
|---|---|---|
| `.github/workflows/release.yml` | 已创建（validate → macos → android → release） | — |
| macOS / Android 打包与 manifest/SHA256SUMS/许可证脚本 | 已创建（`scripts/release/`） | — |
| `Cargo.toml` 头部残留的 `qqmusic-helper-next-cli` 注释 | 已修正 | — |
| NeuMusic 切换 lock + Release 下载 | 未切换 | 本仓库首个正式 Release；宿主侧实现见 §12 |
| 消费端 `helpernext.lock.json` 约定 | 未落地 | 宿主仓库侧实现，见 §12 |

自动发布落地后，分发方式仍维持现状：NeuMusic 从本仓库构建产物接入，macOS 宿主使用本地构建的
stdio 二进制；改为「下载 Release 资产 + 校验 SHA256」是宿主侧的下一步（§12）。§9 的
tag/clean-tree/成套纪律对任何分发方式同样生效。

---

## 结论

当前阶段 Release 只需要把两条正式消费链做好：

```text
1. macOS ARM64 → qqmusic-helper-next stdio binary
2. Android → BoltFFI Kotlin + 四 ABI native libraries
```

Windows、Linux、Apple FFI 与 wasm 尚未适配验证，不进入正式 Release。

比「发布尽可能多的平台二进制」更重要的是这条等式始终成立：

```text
一个 tag = 一个 clean commit = 一个组件版本
        = 一整套不可拆配的 FFI artifact = 一组可验证的 SHA256
```
