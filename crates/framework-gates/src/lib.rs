//! 空库——本 crate 只有 `tests/` 里的仓库级门禁。
//!
//! 为什么单独成一个 crate：门禁审计的是**跨 crate 的仓库属性**（依赖方向、
//! 全局态），属于任何一个具体 crate 都不合适。它刻意零依赖：一旦它依赖了被
//! 审计的 crate，就成不了独立审计。
//!
//! 门禁清单：
//! - `tests/no_core_dependency.rs`：框架不反向依赖宿主（只有 sdk → api 一条边）
//! - `tests/no_global_state.rs`：库代码零进程级单例
//!
//! 领域词汇门禁暂不在此：`echo-plugin-api` 的协议仍带领域分类贡献
//! （`Contribution::Tool` / `Skill` / `PluginInstance`，去业务化排在 M5）。
//! 宿主侧的那条门禁（EchoAgentCore `tests/framework_domain_agnostic.rs`）
//! 覆盖纯机制 crate，随 M3 六件套迁入后搬到这里。
