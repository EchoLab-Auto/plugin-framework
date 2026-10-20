# plugin-framework

业务无关的 Rust 插件化框架：**插件契约 + 插件侧运行时**。

本仓是 [EchoAgentCore](https://github.com/EchoLab-Auto/EchoAgentCore) 插件机制的
框架部分，目标是「任何 Rust 应用都能拿它建一套插件系统」——框架不依赖任何应用
领域词汇（没有 session / tool / llm 这类概念）。EchoAgent 只是它的第一个使用者。

## 状态：六件套齐备（v0.2.0）

| crate | 作用 |
|---|---|
| [`echo-context`](./crates/echo-context) | **内核**：服务定位 `Ctx` + 强类型 `ServiceKey` + 四模式 `EventBus` + 可逆注册 `Disposer` + `ScopedRegistry` |
| [`echo-plugin-api`](./crates/echo-plugin-api) | **冻结协议**：`Hello` / `Invoke` / `Register` / `Cancel` / `Drain` / `HostCall`；贡献类型；版本协商 |
| [`echo-plugin-loader`](./crates/echo-plugin-loader) | 分层配置合成（profile → bundle → patch）+ provenance + 依赖排序 |
| [`echo-plugin`](./crates/echo-plugin) | manifest + 生命周期 + registry（挂载/卸载/启停门控） |
| [`echo-plugin-host`](./crates/echo-plugin-host) | **宿主角**：运输层（inproc / stdio）+ supervisor（握手/取消/排空/崩溃重启）+ conformance 套件 |
| [`echo-plugin-sdk`](./crates/echo-plugin-sdk) | **插件侧运行时**：帧读写、握手、serve 循环、宿主回呼 |
| [`framework-gates`](./crates/framework-gates) | 仓库自守门禁（不反向依赖宿主 / 零全局态 / 领域无关） |

```bash
cargo test --workspace   # 165 项
```

**零全局态现在是无条件成立的**（v0.3.0）：原 `echo-context::kernel`（进程级
单例表）已搬到宿主侧。理由：使用者全在应用层（8 处），框架一处不用；框架导出
进程级单例表会直接破坏「同一进程能起两套宿主」这个复用前提。

**唯一尚未达标的点**：协议表面仍带领域分类贡献（`Contribution::Tool` /
`Skill` / `PluginInstance`），去业务化排在 M5 且有兼容门槛（需 2 个以上真实
外部使用者）。

> 文档注释里少量 `document/design/*.md` 路径指向 EchoAgentCore 仓库——那是本仓
> 前身所在地，第三方文档随 M4 迁来。

## 写一个插件

依赖两行：

```toml
[dependencies]
echo-plugin-sdk = { git = "https://github.com/EchoLab-Auto/plugin-framework", tag = "v0.1.0" }
tokio = { version = "1", features = ["full"] }
```

实现 `PluginHandler`（或 `InstancePlugin`），其余交给 SDK：

```rust
use echo_plugin_sdk::{
    async_trait, serve_stdio, Contribution, InvokeOutcome, InvokeRequest, PluginHandler,
};

struct MyPlugin;

#[async_trait]
impl PluginHandler for MyPlugin {
    fn contributions(&self) -> Vec<Contribution> { vec![] }
    async fn invoke(&self, _req: InvokeRequest) -> InvokeOutcome {
        InvokeOutcome::ok("ok")
    }
}

#[tokio::main]
async fn main() {
    if let Err(e) = serve_stdio(MyPlugin).await {
        eprintln!("plugin exited: {e}");
        std::process::exit(1);
    }
}
```

SDK 承担的东西（自己写一遍大约 450 行，且每处都容易错）：

- **帧读写**：4 字节小端 `u32` 长度前缀 + JSON，读侧容忍分段到达
- **握手**：`Hello` → 校验协议名/版本 → `Welcome` + `Register` + `Ready`；不兼容时
  写 `Failed` 并退出（不是静默跑起来）
- **并发**：每个 `Invoke` 一个任务，结果经内部 mpsc 交由**单写者**串行写出——
  帧不会交错
- **生命周期**：`Cancel` / `Drain` / `Dispose` / stdin EOF 的语义
- **回呼**：`HostClient` 向宿主注册的服务发起 `HostCall`（默认 30s 超时）

约定：**帧只走 stdout，插件日志走 stderr**。

## 版本与兼容

- 本仓**单一版本号**（workspace 共享）。0.x 期间允许 breaking，记入
  [CHANGELOG.md](./CHANGELOG.md)。
- `PROTOCOL_VERSION`（线上契约版本）与 crate 版本**独立**：
  - 改线上帧格式 → bump `PROTOCOL_VERSION`，握手拒绝旧端（fail-closed）
  - 只改 Rust API 表面 → bump crate 版本即可
  - 新增字段一律 `#[serde(default)]`，两端可互换解码
- 消费方建议用 `tag = "v0.x.y"` 钉住；Cargo.lock 会进一步钉死 rev。

## 开发

```bash
cargo test        # 24 项：api 10 + sdk 14（+ 1 doctest）
cargo clippy --all-targets -- -D warnings
```

## License

MIT（见 [LICENSE](./LICENSE)）。
