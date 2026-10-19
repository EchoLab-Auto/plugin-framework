# plugin-framework

业务无关的 Rust 插件化框架：**插件契约 + 插件侧运行时**。

本仓是 [EchoAgentCore](https://github.com/EchoLab-Auto/EchoAgentCore) 插件机制的
框架部分，目标是「任何 Rust 应用都能拿它建一套插件系统」——框架不依赖任何应用
领域词汇（没有 session / tool / llm 这类概念）。EchoAgent 只是它的第一个使用者。

## 状态：**M1（进行中）**

迁移分六个里程碑，当前本仓只包含**两件套**：

| crate | 作用 | 状态 |
|---|---|---|
| [`echo-plugin-api`](./crates/echo-plugin-api) | **冻结协议**：`Hello` / `Invoke` / `Register` / `Cancel` / `Drain` / `HostCall`；贡献类型；版本协商 | ✅ 已迁入 |
| [`echo-plugin-sdk`](./crates/echo-plugin-sdk) | **插件侧运行时**：帧读写、握手、serve 循环、宿主回呼 | ✅ 已迁入 |
| `echo-plugin` | manifest + 生命周期 + registry | ⏳ M3 |
| `echo-plugin-loader` | 分层配置合成（profile → bundle → patch） | ⏳ M3 |
| `echo-plugin-host` | 运输层（inproc / stdio）+ supervisor + conformance 套件 | ⏳ M3 |
| `echo-context` | 服务定位 `Ctx` + 类型化 `EventBus` + 可逆注册 | ⏳ M3 |

> **M3 之前，宿主侧不在本仓。** 想跑一个完整例子，请看
> [EchoAgentCore](https://github.com/EchoLab-Auto/EchoAgentCore) 的
> `source/plugin/echo-plugin-host/`（它已按同一协议实现）。少量文档注释里引用的
> `document/design/*.md` 路径也指向那个仓库，随 M4 的第三方文档一并迁来。

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
