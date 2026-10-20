//! Plugin trait — the mount/unmount lifecycle seam.

use std::sync::Arc;

use echo_context::Disposer;

use crate::manifest::PluginManifest;
/// What a plugin may consume when mounting.
///
/// **只携带框架自己能解释的东西**：一个服务定位器 `Ctx`，加上**本插件的
/// 配置**（`plugins.toml` 里 `[plugin.config]` 原样透传的 JSON）。框架不认识
/// "工具""技能"这类词汇——它们是**宿主的领域概念**，由宿主侧适配层在注册表
/// 里解释。
///
/// 历史（2026-10 M2）：这里曾有 `tools` / `skills` 两个字段，各配一个
/// `PluginToolSink` / `PluginSkillSink` trait。审计发现**零使用**——全仓
/// `mount` 闭包都是 `|_ctx| Ok(vec![])`，`with_skills` 从未被调用，
/// `register_tool` 零调用点。它们是"给未来外部工具插件预留的接缝"，
/// 而预留方式恰好把应用领域词汇写进了框架。删除后框架层零领域依赖。
///
/// 历史（2026-11 配置下发）：`config` 是**框架自己就能解释**的东西——它只是
/// 「一行配置」的 JSON 投影，不含任何领域词汇。此前 in-process 插件拿不到
/// 自己的配置（`plugins.toml` 的 `[plugin.config]` 只送到子进程插件的
/// `Hello.config`），导致"改配置不改代码"对内置插件不成立。
///
/// [`Ctx::provide`]: echo_context::Ctx::provide
#[derive(Default, Clone)]
pub struct MountContext {
    /// Global service locator (services resolved by stable key).
    pub ctx: Option<Arc<echo_context::Ctx>>,
    /// 本插件的配置（`plugins.toml` 的 `[plugin.config]` 对应的 JSON 对象；
    /// 未配置 = [`serde_json::Value::Null`]）。
    ///
    /// **内容由宿主与插件自行约定**，框架只按 `id` 分发、不看内部：
    /// - 缺省 `Null` 而不是 `{}`，这样插件能区分「没配」与「配了空表」；
    /// - 只读（`&self` 上的方法拿到的是 clone），插件改配置请走自己的
    ///   持久化面，别指望改这里能回写。
    pub config: serde_json::Value,
}

impl MountContext {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_ctx(mut self, ctx: Arc<echo_context::Ctx>) -> Self {
        self.ctx = Some(ctx);
        self
    }
    /// 附带本插件的配置（组合根的 `PluginRegistry` 在 mount 时按 id 注入）。
    pub fn with_config(mut self, config: serde_json::Value) -> Self {
        self.config = config;
        self
    }
    /// 本插件的配置（未配置时为 `Null`）。
    pub fn config(&self) -> &serde_json::Value {
        &self.config
    }
    /// 取配置里的一个字符串字段（`null` / 缺失 / 非字符串一律 `None`）。
    ///
    /// 给"只改一两项"的插件省掉重复的 `as_str` 样板——注意它**不**做类型
    /// 校验报告，配错类型等同未配置（要严格校验请自行读 [`Self::config`]）。
    pub fn config_str(&self, key: &str) -> Option<&str> {
        self.config.get(key).and_then(|v| v.as_str())
    }
}

/// Mount error kinds.
#[derive(Debug, thiserror::Error)]
pub enum PluginMountError {
    #[error("mount failed: {0}")]
    Mount(String),
    /// 插件声明的服务键在 `Ctx` 里**不存在**——依赖未就绪。
    ///
    /// 单独成一类（而不是塞进 `Mount(String)`）是为了让宿主能原样转述
    /// "缺什么、有什么"：`available` 是该 `Ctx` 现有的全部服务键。
    #[error("required service `{key}` is not registered (available: {available:?})")]
    MissingService {
        /// 缺失的服务键。
        key: String,
        /// 当前 `Ctx` 里可用的服务键（排查用）。
        available: Vec<String>,
    },
    #[error("plugin kind {kind} is not supported by this host")]
    UnsupportedKind { kind: &'static str },
}

/// Result of a successful mount: disposers that undo the registration.
pub type PluginMountResult = Result<Vec<Disposer>, PluginMountError>;

/// A mountable plugin.
pub trait Plugin: Send + Sync {
    fn manifest(&self) -> &PluginManifest;

    /// Mount the plugin: perform all registrations, returning disposers.
    /// The registry holds these and calls them on unmount.
    fn mount(&self, ctx: &MountContext) -> PluginMountResult;

    /// Extra cleanup beyond disposers (default: nothing).
    fn unmount(&self) -> Result<(), PluginMountError> {
        Ok(())
    }

    /// 本插件**要求的服务键**（`Ctx` 命名空间里的名字；默认空 = 无要求）。
    ///
    /// 语义是「挂我之前，这些服务必须已经在 `Ctx` 里」——注册表在
    /// [`PluginRegistry::mount`] 里逐个 `Ctx::contains` 校验，缺失即
    /// [`PluginMountError::MissingService`]，插件**不会被挂载**。
    ///
    /// 为什么在框架层：服务键是 `echo-context` 的概念（字符串 + 存在性），
    /// 不含任何应用领域词汇；而"依赖缺失就不启动"是插件机制的基本语义
    /// （dsh 的 `inject`）。此前内联插件只能靠组合根手工编排启动顺序，
    /// 顺序错了没有任何反馈——插件照样"挂载成功"，直到运行期才 NPE。
    ///
    /// [`PluginRegistry::mount`]: crate::PluginRegistry::mount
    fn required_services(&self) -> Vec<String> {
        Vec::new()
    }
}

/// A ready-made plugin for built-in modules: manifest + a mount closure.
pub struct BuiltinPlugin {
    manifest: PluginManifest,
    mount_fn: Box<dyn Fn(&MountContext) -> PluginMountResult + Send + Sync>,
    required_services: Vec<String>,
}

impl BuiltinPlugin {
    pub fn new(
        manifest: PluginManifest,
        mount_fn: impl Fn(&MountContext) -> PluginMountResult + Send + Sync + 'static,
    ) -> Self {
        Self {
            manifest,
            mount_fn: Box::new(mount_fn),
            required_services: Vec::new(),
        }
    }

    /// 声明本插件要求的服务键（见 [`Plugin::required_services`]）。
    pub fn with_required_services<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.required_services = keys.into_iter().map(Into::into).collect();
        self
    }
}

impl Plugin for BuiltinPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn mount(&self, ctx: &MountContext) -> PluginMountResult {
        (self.mount_fn)(ctx)
    }

    fn required_services(&self) -> Vec<String> {
        self.required_services.clone()
    }
}
