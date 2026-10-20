# Changelog

本仓遵循 [语义化版本](https://semver.org/lang/zh-CN/)。0.x 期间允许 breaking，
但每一处都记在这里。`PROTOCOL_VERSION` 是**线上契约**版本，与 crate 版本独立
（判定规则见 [README](./README.md#版本与兼容)）。

## [0.2.0] — 2026-10-19

迁入宿主侧四件套，**六件套齐备**——本仓现在可以独立跑通端到端插件链路。
从 EchoAgentCore 逐字节搬迁，代码与行为零改动。

### 新增

- `echo-context` 0.1.0：`Ctx` 服务定位、强类型 `ServiceKey`、四模式 `EventBus`
  （Observe / Waterfall / Parallel / Serial）、可逆注册 `Disposer`、
  `ScopedRegistry`
- `echo-plugin` 0.1.0：manifest + 生命周期 + registry。M2 已去业务化
  ——`kind` 是开放标签（`String`），`MountContext` 只剩 `ctx`
- `echo-plugin-loader` 0.1.0：profile → bundle → patch 三层配置合成 + provenance
  + 依赖排序
- `echo-plugin-host` 0.1.0：`Transport`（inproc / stdio）+ `PluginSupervisor`
  （握手 / 调用 / 取消 / 排空 / 崩溃重启 / 资源限额）+ conformance 套件
  （inproc 14 + stdio 10 + upgrade 10 + bootstrap 9 + host_call 3）
- `framework-gates`：新增 `domain_agnostic` 门禁（4 项）

### 变更

- `no_global_state` 门禁改为**精确文件豁免**：`crates/echo-context/src/kernel.rs`
  是宿主侧应用单例的收敛点（8 处调用全在应用层），终态应搬到宿主侧；白名单
  数量被 `whitelist_stays_minimal` 锁死为 1，棘轮不松

### 已知边界

- 协议贡献类型仍带领域分类（`Contribution::Tool` / `Skill` / `PluginInstance`，
  wire tag 保留历史名 `adapter`），去业务化排在 M5，需 2 个以上真实外部使用者
  才启动
- `echo-context::kernel` 的搬迁（见上）

## [0.1.1] — 2026-10-19

只加**仓库自守门禁**，api / sdk 的代码与协议一字未改（无 API 变化）。

### 新增

- `crates/framework-gates`（零依赖，仅测试）：
  - `no_core_dependency`：框架 crate 只能有 `sdk → api` 一条 echo-* 依赖边；
    出现 `echo-defs` 这类领域 crate 即红
  - `no_global_state`：库代码零进程级单例（`static` / `OnceLock` / `LazyLock` /
    `lazy_static` / `thread_local` / `static mut`）
  - 两条各带自检，防止门禁退化成恒真

### 为什么

框架 crate 从 EchoAgentCore 迁出后，宿主仓的 `no_global_state` 与依赖方向扫描
以那边的 `source/` 为根——**保护随迁移静默消失**。本仓必须自建等价断言，否则
「框架不碰应用」只是文档里的一句话。

两条门禁都实测过能真报错（注入违规 → FAILED；撤销 → 全绿）。

## [0.1.0] — 2026-10-19

首个版本，从 EchoAgentCore 迁出（`source/plugin/echo-plugin-{api,sdk}`），
代码与行为**零改动**——本次发布只是让它们能被外部独立编译。

### 包含

- `echo-plugin-api` 0.1.0：冻结协议。协议名 `echo-plugin`，`PROTOCOL_VERSION = 1`。
  - 消息：`Hello` / `Welcome` / `Register` / `Ready` / `Invoke` / `InvokeResult` /
    `Cancel` / `Drain` / `Dispose` / `Emit` / `Event` / `Log` / `HostCall` /
    `HostCallResult` / `Failed`
  - 贡献类型：`Tool` / `Skill` / `Service` / `Events` / `PluginInstance`
    （wire tag 为 `adapter`，为兼容已部署插件而保留）
  - 能力位：`tools` / `skills` / `services` / `events` / `cancel` / `host_call` / `adapter`
- `echo-plugin-sdk` 0.1.0：插件侧运行时（`serve_stdio` / `serve_io` /
  `InstancePlugin` / `InstanceHandler` / `HostClient`），re-export `echo_plugin_api`。

### 已知边界

- 宿主侧（`echo-plugin-host` / `echo-plugin` / `echo-plugin-loader`）与 `echo-context`
  **尚未迁入**，计划 M3。在此之前本仓不能单独跑通端到端示例。
- `Contribution` 枚举仍包含应用领域分类（`Tool` / `Skill` / `PluginInstance`）。
  机制本身与领域无关，但协议表面承认了这些分类；去业务化留待 M5 评估。
