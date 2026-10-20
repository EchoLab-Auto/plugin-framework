//! 插件树：作用域（`Ctx` 父子）+ 归属（谁挂的子插件）+ 级联卸载。
//!
//! 一个 [`PluginScope`] = **一层**：一个 [`Ctx`] 作用域 + 一个
//! [`PluginRegistry`]。根作用域的 ctx 就是宿主的服务表；子作用域的 ctx 是
//! 它的 [`Ctx::child`]（父链继承、子遮蔽父）。
//!
//! ## 与 dsh 的对应
//!
//! dsh 里 `ctx.plugin(Foo)` 把 Foo 挂成**当前上下文的子插件**：Foo 自己的
//! `ctx` 继承父的服务、但它注册的东西不污染父；父被 dispose 时子级联退出。
//! 本模块给的是同一套结构的最小型：
//!
//! | dsh | 这里 |
//! |---|---|
//! | `ctx.plugin(Foo)` | `scope.mount(Foo)`（或插件在 mount 里 `mctx.nest()`） |
//! | 子 ctx 继承父 | `Ctx::child`（父链查找 + 遮蔽） |
//! | 父 dispose → 子 dispose | `unmount(父)` → 该插件名下子作用域 `shutdown()` |
//! | `ctx.isolate()` | `Ctx::child` |
//!
//! ## 归属如何记录
//!
//! 注册表在 `mount(id)` 期间把 id 压进 [`TreeLinks::mounting`]，插件在
//! mount 里调用 [`MountContext::nest`] 时，新建的子作用域就记在
//! **栈顶那个 id** 名下。于是"这个子作用域属于谁"不需要插件自报，也不会
//! 因为插件挂别人的子作用域而串味。
//!
//! [`MountContext::nest`]: crate::MountContext::nest

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use echo_context::Ctx;

use crate::plugin::{MountContext, PluginMountError, PluginMountResult};
use crate::registry::PluginRegistry;
use crate::Plugin;

/// 一层作用域的内部归属表（`PluginScope` 与它的注册表共享）。
#[derive(Default)]
pub(crate) struct TreeLinks {
    /// 当前正在挂载的插件 id（栈；嵌套挂载时非空）。
    mounting: Mutex<Vec<String>>,
    /// `owner 插件 id` → 它在挂载期建出的子作用域。
    children: Mutex<BTreeMap<String, Vec<Arc<PluginScope>>>>,
}

impl TreeLinks {
    fn enter(&self, id: &str) {
        self.mounting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(id.to_string());
    }

    fn leave(&self, id: &str) {
        let mut stack = self
            .mounting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // 只弹栈顶自己：中间被别的 mount 压进来时不该误弹。
        if stack.last().map(String::as_str) == Some(id) {
            stack.pop();
        } else if let Some(pos) = stack.iter().rposition(|entry| entry == id) {
            stack.remove(pos);
        }
    }

    pub(crate) fn current_owner(&self) -> Option<String> {
        self.mounting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last()
            .cloned()
    }

    fn adopt(&self, owner: &str, child: Arc<PluginScope>) {
        self.children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(owner.to_string())
            .or_default()
            .push(child);
    }

    pub(crate) fn take_children(&self, owner: &str) -> Vec<Arc<PluginScope>> {
        self.children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(owner)
            .unwrap_or_default()
    }

    pub(crate) fn children_of(&self, owner: &str) -> Vec<Arc<PluginScope>> {
        self.children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(owner)
            .cloned()
            .unwrap_or_default()
    }

    fn total_children(&self) -> usize {
        self.children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .map(Vec::len)
            .sum()
    }
}

/// 一层作用域：独立 `Ctx` + 独立插件注册表 + 归属关系。
///
/// 用 [`PluginScope::root`] 建根，用 [`PluginScope::nest`] /
/// [`MountContext::nest`] 建子。卸载父插件会级联卸载它名下的子作用域。
///
/// [`MountContext::nest`]: crate::MountContext::nest
pub struct PluginScope {
    ctx: Arc<Ctx>,
    registry: PluginRegistry,
    depth: usize,
    links: Arc<TreeLinks>,
}

impl std::fmt::Debug for PluginScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginScope")
            .field("depth", &self.depth)
            .field("ids", &self.registry.ids())
            .field("children", &self.links.total_children())
            .finish()
    }
}

impl PluginScope {
    /// 建**根作用域**（`ctx` 就是宿主的服务表）。
    pub fn root(ctx: Arc<Ctx>) -> Arc<Self> {
        Self::build(ctx, 0)
    }

    fn build(ctx: Arc<Ctx>, depth: usize) -> Arc<Self> {
        let links = Arc::new(TreeLinks::default());
        let scope = Arc::new(Self {
            ctx,
            registry: PluginRegistry::new(),
            depth,
            links,
        });
        // 注册表通过弱引用回指本作用域：卸载时据此级联卸子作用域。
        // 用 Weak 而不是 Arc——`scope → registry → scope` 成环会泄漏。
        scope.registry.attach_tree(Arc::downgrade(&scope));
        scope
    }

    /// 本作用域的服务表（子作用域是父的 child，父服务对它可见）。
    pub fn ctx(&self) -> &Arc<Ctx> {
        &self.ctx
    }

    /// 本作用域自己的插件注册表。
    pub fn registry(&self) -> &PluginRegistry {
        &self.registry
    }

    /// 距根的深度（根 = 0）。
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// 本作用域已注册的插件 id（注册顺序）。
    pub fn ids(&self) -> Vec<String> {
        self.registry.ids()
    }

    /// 挂一个插件到本作用域（register + mount，立即校验注入）。
    ///
    /// 与直接用 `registry.register_and_mount` 的区别：这里的挂载上下文带着
    /// **本作用域**（`MountContext::nest` 可用），并且本插件在 mount 里建的
    /// 子作用域会记在它名下——卸载它时级联卸载。
    pub fn mount(&self, plugin: Arc<dyn Plugin>) -> Result<(), String> {
        let id = plugin.manifest().id.clone();
        self.registry.register(plugin)?;
        self.registry
            .mount(&id, &self.mount_context())
            .map(|_| ())
            .map_err(|e| format!("plugin {id} mount failed: {e}"))
    }

    /// 挂一个插件，先等它的 inject 就绪（最多 `timeout`）。
    pub async fn mount_when_ready(
        &self,
        plugin: Arc<dyn Plugin>,
        timeout: Duration,
    ) -> Result<(), String> {
        let id = plugin.manifest().id.clone();
        self.registry.register(plugin)?;
        self.registry
            .mount_when_ready(&id, &self.mount_context(), timeout)
            .await
            .map(|_| ())
            .map_err(|e| format!("plugin {id} mount failed: {e}"))
    }

    /// 卸一个插件（级联卸掉它名下所有子作用域，子先于父）。
    pub fn unmount(&self, id: &str) -> Result<(), PluginMountError> {
        self.registry.unmount(id)
    }

    /// 用已注册的 id 在本作用域挂载（配置与注入由注册表按 id 处理）。
    pub fn mount_id(&self, id: &str) -> PluginMountResult {
        self.registry.mount(id, &self.mount_context())
    }

    /// 卸掉本作用域的全部插件（子作用域随各自 owner 一并级联）。
    pub fn shutdown(&self) {
        self.registry.unmount_all();
    }

    /// 为**当前正在挂载的插件**建一个子作用域；不在挂载期时 `None`。
    ///
    /// 子作用域的 ctx 是 [`Ctx::child`]（父链继承），注册表独立。
    /// 通常不直接调用——插件侧用 [`MountContext::nest`]。
    ///
    /// [`MountContext::nest`]: crate::MountContext::nest
    pub fn nest(self: &Arc<Self>) -> Option<Arc<PluginScope>> {
        let owner = self.links.current_owner()?;
        let child = Self::build(self.ctx.child(), self.depth + 1);
        self.links.adopt(&owner, child.clone());
        Some(child)
    }

    /// 某插件名下已建出的子作用域（只读视图，用于诊断/测试）。
    pub fn children_of(&self, owner: &str) -> Vec<Arc<PluginScope>> {
        self.links.children_of(owner)
    }

    /// 本作用域名下子作用域总数（不含孙子）。
    pub fn child_scope_count(&self) -> usize {
        self.links.total_children()
    }

    /// 递归描述整棵子树（诊断用）：`[(depth, 插件 id)]`。
    pub fn walk(&self) -> Vec<(usize, String)> {
        let mut out: Vec<(usize, String)> = self
            .registry
            .ids()
            .into_iter()
            .map(|id| (self.depth, id))
            .collect();
        for children in self
            .links
            .children
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
            .values()
        {
            for child in children {
                out.extend(child.walk());
            }
        }
        out
    }

    /// 本作用域的挂载上下文：共享 ctx、带树把手（`MountContext::nest` 可用）。
    /// 压栈"正在挂载的插件"（注册表内部用）。
    pub(crate) fn links_enter(&self, id: &str) {
        self.links.enter(id);
    }

    /// 弹栈（`OwnerGuard::drop`）。
    pub(crate) fn links_leave(&self, id: &str) {
        self.links.leave(id);
    }

    /// 取走某插件名下的子作用域（卸载时级联用）。
    pub(crate) fn links_take_children(&self, owner: &str) -> Vec<Arc<PluginScope>> {
        self.links.take_children(owner)
    }

    fn mount_context(&self) -> MountContext {
        let base = MountContext::new().with_ctx(Arc::clone(&self.ctx));
        match self.registry.tree_handle() {
            Some(tree) => base.with_tree(tree),
            None => base,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::PluginManifest;
    use crate::plugin::BuiltinPlugin;
    use echo_context::Disposer;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 造一个"父插件"：在自己的 mount 里 nest 出子作用域，往里面挂子插件。
    /// 子插件往自己的 ctx 里注册一个服务（父作用域看不见它）。
    fn parent_with_child(
        parent_id: &str,
        child_id: &str,
        parent_mounts: Arc<AtomicUsize>,
        child_mounts: Arc<AtomicUsize>,
        child_disposed: Arc<AtomicUsize>,
    ) -> Arc<dyn Plugin> {
        let manifest = PluginManifest::builtin(parent_id, parent_id, "1", "tool", "", "");
        let child_id = child_id.to_string();
        Arc::new(BuiltinPlugin::new(manifest, move |mctx: &MountContext| {
            parent_mounts.fetch_add(1, Ordering::SeqCst);
            let scope = mctx.nest().expect("有树把手才能 nest");
            let cm = child_mounts.clone();
            let cd = child_disposed.clone();
            let child = Arc::new(BuiltinPlugin::new(
                PluginManifest::builtin(&child_id, &child_id, "1", "tool", "", ""),
                move |cmctx: &MountContext| {
                    cm.fetch_add(1, Ordering::SeqCst);
                    // 子作用域自己的服务：父作用域看不到。
                    let ctx = cmctx.ctx.clone().expect("子作用域有 ctx");
                    let keep = ctx.register::<u8>("child-private", 7u8);
                    let cd2 = cd.clone();
                    Ok(vec![
                        keep,
                        Disposer::from_fn(move || {
                            cd2.fetch_add(1, Ordering::SeqCst);
                        }),
                    ])
                },
            ));
            scope.mount(child).expect("子插件挂载");
            Ok(vec![])
        }))
    }

    /// 父插件 nest 出的子作用域：子插件能看见父的服务，父看不见子的。
    #[test]
    fn child_scope_shadows_and_isolates_services() {
        let root_ctx = Arc::new(Ctx::default());
        let _root_svc = root_ctx.register::<u8>("shared", 1u8);
        let scope = PluginScope::root(root_ctx.clone());
        let plugin = parent_with_child(
            "p.parent",
            "p.child",
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
        );
        scope.mount(plugin).expect("parent mounts");

        let children = scope.children_of("p.parent");
        assert_eq!(children.len(), 1, "父插件名下应有一个子作用域");
        let child = &children[0];
        assert_eq!(child.depth(), 1);
        assert_eq!(child.ids(), vec!["p.child".to_string()]);
        // 父链继承：子能看见根的服务
        assert!(child.ctx().contains("shared"), "子作用域应继承父的服务");
        // 子私有：根/父都看不见
        assert!(child.ctx().contains("child-private"));
        assert!(!root_ctx.contains("child-private"), "子服务不该漏到根");
        assert!(!scope.ctx().contains("child-private"));
        // resolve 也走父链
        assert_eq!(root_ctx.resolve::<u8>("shared"), Some(1));
        assert_eq!(child.ctx().resolve::<u8>("shared"), Some(1), "子沿父链解析");
        // 类型不符不算命中（哪怕键存在）
        assert_eq!(
            child.ctx().resolve::<i32>("shared"),
            None,
            "类型不同不算命中"
        );
    }

    /// 卸载父插件 → 子作用域级联卸载（子先于父，子插件 disposer 执行）。
    #[test]
    fn unmounting_parent_cascades_to_child_scopes() {
        let root_ctx = Arc::new(Ctx::default());
        let scope = PluginScope::root(root_ctx);
        let child_mounts = Arc::new(AtomicUsize::new(0));
        let child_disposed = Arc::new(AtomicUsize::new(0));
        scope
            .mount(parent_with_child(
                "p.parent",
                "p.child",
                Arc::new(AtomicUsize::new(0)),
                child_mounts.clone(),
                child_disposed.clone(),
            ))
            .expect("parent mounts");
        assert_eq!(child_mounts.load(Ordering::SeqCst), 1);

        scope.unmount("p.parent").expect("unmount ok");

        assert_eq!(
            child_disposed.load(Ordering::SeqCst),
            1,
            "子插件应随父卸载而被卸载（disposer 执行）"
        );
        // 子作用域已从归属表移除
        assert!(scope.children_of("p.parent").is_empty());
        assert!(!scope.registry().is_mounted("p.parent"));
    }

    /// 嵌套两层：孙插件的副作用也在祖父卸载时被撤掉（递归级联）。
    #[test]
    fn three_levels_cascade_recursively() {
        let root_ctx = Arc::new(Ctx::default());
        let scope = PluginScope::root(root_ctx);
        let grandchild_mounted = Arc::new(AtomicUsize::new(0));
        let grandchild_disposed = Arc::new(AtomicUsize::new(0));

        // 中层的 mount 里再 nest 一层
        let gc_m = grandchild_mounted.clone();
        let gc_d = grandchild_disposed.clone();
        let middle = Arc::new(BuiltinPlugin::new(
            PluginManifest::builtin("p.middle", "m", "1", "tool", "", ""),
            move |mctx: &MountContext| {
                let inner = mctx.nest().expect("middle nests");
                let gc_m = gc_m.clone();
                let gc_d = gc_d.clone();
                let leaf = Arc::new(BuiltinPlugin::new(
                    PluginManifest::builtin("p.leaf", "l", "1", "tool", "", ""),
                    move |_| {
                        gc_m.fetch_add(1, Ordering::SeqCst);
                        let d = gc_d.clone();
                        Ok(vec![Disposer::from_fn(move || {
                            d.fetch_add(1, Ordering::SeqCst);
                        })])
                    },
                ));
                inner.mount(leaf).expect("leaf mounts");
                Ok(vec![])
            },
        ));
        scope.mount(middle).expect("middle mounts");
        assert_eq!(grandchild_mounted.load(Ordering::SeqCst), 1);

        // walk 应该看到两层嵌套
        let ids: Vec<String> = scope.walk().into_iter().map(|(_, id)| id).collect();
        assert!(
            ids.contains(&"p.leaf".to_string()),
            "walk 应递归到孙子: {ids:?}"
        );

        scope.unmount("p.middle").expect("unmount ok");
        assert_eq!(
            grandchild_disposed.load(Ordering::SeqCst),
            1,
            "孙插件副作用应随中层卸载递归撤销"
        );
    }

    /// 裸注册表（无作用域）挂载同一个父插件：`nest()` 返回 None，
    /// 但挂载本身不能因此失败——树是**可选**能力。
    #[test]
    fn plugin_without_tree_cannot_nest_but_still_mounts() {
        let reg = PluginRegistry::new();
        let nested = Arc::new(AtomicUsize::new(0));
        let n = nested.clone();
        reg.register(Arc::new(BuiltinPlugin::new(
            PluginManifest::builtin("p.bare", "b", "1", "tool", "", ""),
            move |mctx: &MountContext| {
                assert!(mctx.nest().is_none(), "裸注册表没有树把手");
                n.fetch_add(1, Ordering::SeqCst);
                Ok(vec![])
            },
        )))
        .unwrap();
        let ctx = MountContext::new().with_ctx(Arc::new(Ctx::default()));
        reg.mount("p.bare", &ctx).expect("mount ok");
        assert_eq!(nested.load(Ordering::SeqCst), 1);
    }

    /// 子作用域的服务注册**沿父链向上通知**：根上的订阅者能看见子树变动。
    #[test]
    fn change_notifications_bubble_up_to_the_root() {
        use echo_context::Change;
        use std::sync::Mutex;

        let root_ctx = Arc::new(Ctx::default());
        let seen: Arc<Mutex<Vec<Change>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let _sub = root_ctx.on_change(move |change| sink.lock().unwrap().push(change));

        let scope = PluginScope::root(root_ctx.clone());
        scope
            .mount(parent_with_child(
                "p.parent",
                "p.child",
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicUsize::new(0)),
            ))
            .expect("mount");

        let changes = seen.lock().unwrap().clone();
        let provided: Vec<(String, usize)> = changes
            .iter()
            .filter_map(|c| match c {
                Change::Provided { key, depth } => Some((key.clone(), *depth)),
                _ => None,
            })
            .collect();
        assert!(
            provided
                .iter()
                .any(|(key, depth)| key == "child-private" && *depth == 1),
            "根的订阅者应收到子作用域（depth=1）的注册: {provided:?}"
        );
    }
}
