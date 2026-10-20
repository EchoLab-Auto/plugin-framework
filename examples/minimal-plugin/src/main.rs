//! 最小插件（standalone binary，走 stdio 协议）。
//!
//! 这个 crate **只依赖 `echo-plugin-sdk`**——不依赖任何宿主应用的东西。它证明
//! 了「写一个插件」不需要理解宿主是什么：实现 `PluginHandler`，交给
//! `serve_stdio`，剩下的（帧、握手、并发、单写者、生命周期）由 SDK 承担。
//!
//! 贡献用 `Contribution::Service`（`key` + `kind` + `version`）而不是
//! `Tool` / `Skill`——后者是宿主应用的领域分类，前者是框架的通用机制。
//! 宿主怎么解释这个 key，是宿主的事。
//!
//! 运行（由宿主 spawn，不手工跑）：
//! ```text
//! cargo run -p minimal-host
//! ```

use echo_plugin_sdk::echo_plugin_api::ServiceContribution;
use echo_plugin_sdk::{
    async_trait, serve_stdio, Contribution, InvokeOutcome, InvokeRequest, PluginHandler,
};
use serde_json::{json, Value};

/// 服务键：宿主按这个名字调用（`Invoke("greeter")`）。
const SERVICE_KEY: &str = "greeter";

struct Greeter;

#[async_trait]
impl PluginHandler for Greeter {
    fn contributions(&self) -> Vec<Contribution> {
        vec![Contribution::Service(ServiceContribution {
            key: SERVICE_KEY.to_string(),
            kind: "greeter".to_string(),
            version: 1,
        })]
    }

    async fn invoke(&self, req: InvokeRequest) -> InvokeOutcome {
        // SDK 已校验 `req.contribution` 在 contributions 之内，这里只需处理业务。
        match req.contribution.as_str() {
            SERVICE_KEY => {
                let name = req
                    .payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("world");
                InvokeOutcome::ok(json!({ "greeting": format!("hello, {name}") }).to_string())
            }
            other => InvokeOutcome::Error {
                code: "unknown_contribution".to_string(),
                message: format!("unknown contribution: {other}"),
            },
        }
    }
}

#[tokio::main]
async fn main() {
    // 日志必须走 stderr——stdout 是协议通道，混入任何字节都会破坏帧。
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    if let Err(error) = serve_stdio(Greeter).await {
        eprintln!("minimal-plugin exited: {error}");
        std::process::exit(1);
    }
}
