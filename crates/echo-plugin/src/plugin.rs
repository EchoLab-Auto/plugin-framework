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
    /// 插件树把手：本插件**挂在哪个作用域**里（`None` = 裸注册表，无树）。
    ///
    /// 有它时插件可以在自己的 `mount` 里 [`MountContext::nest`] 出一个子
    /// 作用域，把子插件挂进去——卸载本插件时子作用域连带卸载（级联）。
    /// 这是 dsh `ctx.plugin()` 嵌套的对应物；**框架不认识"子插件"这个业务
    /// 概念**，只认"作用域归属"这一层结构。
    pub tree: Option<Arc<crate::PluginScope>>,
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
    /// 附带插件树把手（`PluginScope::mount` 在挂载前注入）。
    pub fn with_tree(mut self, tree: Arc<crate::PluginScope>) -> Self {
        self.tree = Some(tree);
        self
    }
    /// 本插件所在的作用域（裸注册表挂载时为 `None`）。
    pub fn tree(&self) -> Option<&Arc<crate::PluginScope>> {
        self.tree.as_ref()
    }
    /// 为**本插件**建一个子作用域（子插件挂这里）；无树可用时 `None`。
    ///
    /// 只在 `mount` 期间有效（作用域要知道当前挂载的是谁，才能把子作用域
    /// 记在它名下、随它卸载而级联卸掉）。返回的作用域有独立的 `Ctx`
    /// （父 = 本插件所在作用域的 ctx）与独立注册表。
    pub fn nest(&self) -> Option<Arc<crate::PluginScope>> {
        self.tree.as_ref()?.nest()
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

/// 声明式依赖：本插件要用哪些服务键（dsh 的 `inject`）。
///
/// 两种语义，都由宿主实现：
///
/// - **required**（硬依赖）：挂载前必须都在。缺失 → 拒绝挂载
///   （[`PluginMountError::MissingService`]，mount 闭包不执行）；
/// - **optional**（软依赖）：有则用、无则降级，只作为**声明**记录下来
///   （面板展示 / 宿主按需读），不参与挂载判定。
///
/// 为什么是服务键而不是类型：键的**存在性**是 `echo-context` 的概念，
/// 键背后的类型是宿主与插件的私事。框架认类型就得认识宿主的 trait，
/// `echo-plugin` 零领域依赖的边界立刻破掉（见 `framework-gates`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inject {
    /// 硬依赖：缺失即拒绝挂载。
    pub required: Vec<String>,
    /// 软依赖：仅声明（缺失不阻止挂载）。
    pub optional: Vec<String>,
}

impl Inject {
    /// 只要硬依赖。
    pub fn new<I, S>(required: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            required: required.into_iter().map(Into::into).collect(),
            optional: Vec::new(),
        }
    }

    /// 追加软依赖。
    pub fn with_optional<I, S>(mut self, optional: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.optional.extend(optional.into_iter().map(Into::into));
        self
    }

    /// 硬依赖里当前**缺失**的键（`ctx` 为 `None` 时全部算缺失）。
    pub fn missing(&self, ctx: Option<&Arc<echo_context::Ctx>>) -> Vec<String> {
        match ctx {
            Some(ctx) => self
                .required
                .iter()
                .filter(|key| !ctx.contains(key))
                .cloned()
                .collect(),
            None => self.required.clone(),
        }
    }

    /// 硬依赖是否全部就绪。
    pub fn is_ready(&self, ctx: Option<&Arc<echo_context::Ctx>>) -> bool {
        self.missing(ctx).is_empty()
    }

    /// 是否什么都没声明。
    pub fn is_empty(&self) -> bool {
        self.required.is_empty() && self.optional.is_empty()
    }
}

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

    /// 本插件的**声明式依赖**（服务键；默认空 = 无要求）。
    ///
    /// 这是插件表达「我需要什么」的唯一声明面：
    /// - [`PluginRegistry::mount`] 立即校验 required，缺失即
    ///   [`PluginMountError::MissingService`]，**mount 闭包不执行**；
    /// - [`PluginRegistry::mount_when_ready`] 先等它就绪（带超时）再挂；
    /// - [`PluginRegistry::reconcile`] 在依赖变动后据此重挂 / 卸下。
    ///
    /// 为什么在框架层：服务键是 `echo-context` 的概念（字符串 + 存在性），
    /// 不含任何应用领域词汇；"依赖没到就不启动、到了就启动"是插件机制的
    /// 基本语义（dsh 的 `inject`）。此前内联插件只能靠组合根手工编排启动
    /// 顺序，顺序错了毫无反馈——插件照样"挂载成功"，直到运行期才 NPE。
    ///
    /// [`PluginRegistry::mount`]: crate::PluginRegistry::mount
    /// [`PluginRegistry::mount_when_ready`]: crate::PluginRegistry::mount_when_ready
    /// [`PluginRegistry::reconcile`]: crate::PluginRegistry::reconcile
    fn inject(&self) -> Inject {
        Inject::default()
    }

    /// 硬依赖的键（[`Self::inject`] 的便捷读法：`self.inject().required`）。
    ///
    /// 保留它是**兼容 v0.5.0 的接缝**：那一版只有 `required_services`，
    /// 现有实现覆写的是它。默认实现读 `inject().required`，所以两种写法
    /// 都生效、且只有一个真值。
    fn required_services(&self) -> Vec<String> {
        self.inject().required
    }
}

/// A ready-made plugin for built-in modules: manifest + a mount closure.
pub struct BuiltinPlugin {
    manifest: PluginManifest,
    mount_fn: Box<dyn Fn(&MountContext) -> PluginMountResult + Send + Sync>,
    inject: Inject,
}

impl BuiltinPlugin {
    pub fn new(
        manifest: PluginManifest,
        mount_fn: impl Fn(&MountContext) -> PluginMountResult + Send + Sync + 'static,
    ) -> Self {
        Self {
            manifest,
            mount_fn: Box::new(mount_fn),
            inject: Inject::default(),
        }
    }

    /// 声明本插件的依赖（见 [`Plugin::inject`]）：硬依赖 + 可选软依赖。
    pub fn with_inject(mut self, inject: Inject) -> Self {
        self.inject = inject;
        self
    }

    /// 只声明硬依赖（**兼容 v0.5.0**：等价于
    /// `with_inject(Inject::new(keys))`）。
    pub fn with_required_services<I, S>(self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.with_inject(Inject::new(keys))
    }
}

impl Plugin for BuiltinPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn mount(&self, ctx: &MountContext) -> PluginMountResult {
        (self.mount_fn)(ctx)
    }

    fn inject(&self) -> Inject {
        self.inject.clone()
    }
}
