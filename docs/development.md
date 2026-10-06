# 开发流程：具体怎么改

本文是改动的操作手册：任务怎么分类、新增能力按什么顺序、哪些事情被禁止。
分层原理见 [architecture.md](architecture.md)，接口契约见 [endpoints.md](endpoints.md) 与
[parsing.md](parsing.md)，验证见 [testing.md](testing.md)。

## 1. 任务分类

| 类型 | 典型改动 | 落点 |
|---|---|---|
| A 文档/规则 | 契约记录、待决事项、发布规则 | `docs/`，发布语义同步 [RELEASING.md](RELEASING.md) |
| B 新增端点（常态） | 参考实现里有、组件还没有的接口 | `src/port/` 新领域文件，按第 4 节检查表 |
| C 既有契约变更（罕见） | 改 `src/api.rs`/`models.rs`/`methods.rs`/`catalog.rs` 或 port 既有文件的既有行为 | 先在 [pending.md](pending.md) 记录取舍，评审后再动；同步 FFI 成套更新 |
| D 平台与打包 | `boltffi.toml`、打包脚本、CI | 遵守 [RELEASING.md](RELEASING.md) 的范围与禁止事项 |
| E 测试基建 | 冒烟脚本、离线测试 | 遵守 [testing.md](testing.md) 的安全规则 |

## 2. 新增能力的固定顺序

1. **先查已有能力**（见第 3 节）——确认真的没有等价入口；
2. 实现：`src/port/<domain>.rs` 一个文件写全 `METHODS`、`dispatch`、`#[data]` 模型、`#[export]` 包装、`#[cfg(test)]` 测试；
3. 登记：`src/port/mod.rs` 三处——`mod` 列表、`all_methods()`、`dispatch()`；
4. 生成绑定：`boltffi generate swift` 与 `boltffi generate kotlin` 成套重跑；
5. 验证：`cargo test` → 绑定生成 →（需要时）`scripts/port-smoke.sh`；
6. 文档：[endpoints.md](endpoints.md) 登记一行；有解析陷阱的进 [parsing.md](parsing.md)；
   未决与验证边界进 [pending.md](pending.md)。

顺序反了（比如先写绑定再登记）只会白跑——漏登记会被 port 层的确定性测试拦下。

## 3. 先查已有能力

新增之前先确认组件里没有同一个端点的第二份实现：

```sh
grep -rn "模块名/方法名" src/methods.rs src/port/   # 例：music.SongInfoBase / GetSongInfo
grep -n "方法英文名" src/port/*/METHODS src/methods.rs
```

- 参考实现里的别名（如 like/unlike→`set_liked`、推荐流/新歌的复用入口）**复用既有方法**，不另开新名字；
- 同一上游端点只允许一个实现；类型化包装只是薄壳，把请求交给协议层再解析成模型；
- 参考快照（`dist/reference/QQMusicApi/`，不进版本库）是 module/method/param 的对错判据。

## 4. 新增 endpoint 检查表（port 层）

- [ ] 新文件 `src/port/<domain>.rs`：`pub const METHODS`、`pub fn dispatch`、`#[data]` 模型、`#[export]` 包装、测试，全在一个文件；
- [ ] `src/port/mod.rs` 三处登记：`mod` 列表、`all_methods()`、`dispatch()`；
- [ ] 每个方法有**同名** `#[export]` 包装（宿主与绑定都按这个名字调），且方法名同时出现在
      `METHODS` 与 `dispatch` 两处——`src/port/mod.rs` 的确定性测试会逐个核对；
- [ ] 平台档案按实测结论选（见第 6 节），调用方显式传 `platform` 时尊重调用方；
- [ ] 错误走 `HelperError`（见第 7 节），缺信封报错、业务空值按 [parsing.md](parsing.md) 的结论归一；
- [ ] `boltffi generate swift` / `kotlin` 成套重跑，无 skipped 函数（有就说明形状生成器不吃，改模型）；
- [ ] `cargo test` 绿；
- [ ] 文档三处同步（endpoints / parsing / pending）。

## 5. 禁止复制 endpoint 实现

- 同一端点出现第二份实现（一份给 stdio、一份给 FFI、一份给某领域）是明确禁止的——两个调用面
  都经 `methods::dispatch`，这是结构保证；
- 需要新形状时改模型或加参数，不开分支实现；
- port 层需要上游时走 `port::call`（共享 `Upstream`），**不得自带 HTTP agent、限流器或熔断器**。

## 6. 公开 API 变更规则

- `#[data]` / `#[export]` 是公开契约：**字段只追加、不重排、不改名**。FFI 按字段顺序编码，
  Swift/Kotlin bindings 与 native library 必须由同一版本成套生成与发布，新旧不能混用（详见 [ffi.md](ffi.md)）；
- `api_surface_matches`（`src/api.rs:555`）强制 `METHODS` 与类型化包装一一对应；
  包装名与协议名不同时（如 `import_cookies`→`import_credential`）登记在测试的别名表里；
- `PROTOCOL_VERSION`（现为 2）只在 stdio 协议形状变化时变动，且必须保持旧字段兼容；
- 生成器限制：`Vec<(String, String)>` 元组向量会失败/被跳过，用具名 `#[data]` 结构替代（见 [ffi.md](ffi.md)）。

## 7. 平台档案规则

- 档案只有两个：`Platform::Web`（默认）与 `Platform::Android`（`src/upstream.rs:38-78`）；
  档案只改 `comm` 叠加，不改凭据；
- **用错档案不报错**：搜索回空 `meta.sum`、取流回 `104003`、部分接口直接风控——所以移植前必须查
  [parsing.md](parsing.md) 第一节的实测结论，不要按「歌手资料走 android」一刀切；
- 部分方法在 `methods.rs` 的 `catalog_dispatch` 里缺省强制 android（写、搜索、推荐流、取流等），
  调用方显式给 `params.platform` 时永远优先；
- 新端点拿不准时：先按参考实现的档案发请求实测，把结论写进 [parsing.md](parsing.md)，再落代码。

## 8. 错误规则

- 全表面一个错误类型 `HelperError`（`src/lib.rs`）：`NotLoggedIn` / `Throttled` / `Upstream` /
  `Unsupported` / `InvalidRequest`；`UpstreamError` 的映射在 `src/lib.rs` 的 `From` 实现；
- 上游业务码非零、响应形状不对 → `HelperError::Upstream`（带上游原文细节，方便排查）；
- 缺信封键是**错误**，不是空默认值：「这个账号没有歌单」是合法答案但必须有信封兜着，
  静默回空列表会覆盖宿主缓存里的真数据；
- 限流/熔断拒绝 → `Throttled`，文案里带等待时长；调用方按 `success`/`throttled` 语义自行决定重试，
  组件不自动重试（含 `1000` 限流码，见 [pending.md](pending.md)）。

## 9. 凭据规则

- 凭据读写只在 `src/credential.rs`（与被替换的 Python helper 同一文件、同一键集，可互换）；
- `g_tk = hash33(qm_keyst)`；cookie 头由组件组装，宿主只给 `uin` + `qm_keyst`
  （可选 `encrypt_uin`，走 `import_credential_with_encrypt_uin`）；
- **凭据与密钥不进 git、不进日志、不进文档、不进测试报告**；stdio 回包与 stderr 日志同样不得出现。

## 10. 格式化政策

- **禁止全仓跑 `cargo fmt`**；CI 不放 `fmt --check`。
- 只允许对本次改动的文件执行：

  ```sh
  rustfmt --edition 2021 src/port/<你改的文件>.rs
  ```

- 理由：全仓重格式化会把无关文件卷进 diff，淹没真实改动、污染 blame 与评审。
