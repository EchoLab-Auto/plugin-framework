---
id: model
title: 核心模型
group: 概念
link: ["crates | 六件套", "semantics | 插件语义"]
---

# 核心模型

整个框架只有五个概念。想清楚这五个，剩下的都是它们的组合。

## 一、`Ctx`：按名字找服务

服务定位器。注册时给类型，解析时按**名字 + 类型**取；类型不符等于没找到。

```rust
let ctx = Arc::new(Ctx::default());
let _keep = ctx.register::<Arc<dyn Redactor>>("sanitizer", redactor);
let got: Option<Arc<dyn Redactor>> = ctx.resolve("sanitizer");
```

强类型那一层是 `ServiceKey<T>`：`provide` / `service` / `require` /
`service_or_wait`，把"名字"和"存进去的确切类型"绑在一起，顺便提供**就绪等待**。

**为什么是字符串键**：插件之间不互相 `use` 对方的类型。一旦要求类型化，
"插件可被独立替换"就不成立了（换掉一个插件就得重新编译所有引用它的插件）。

## 二、`Disposer`：注册就是副作用

每个注册动作都返回一个撤销句柄：

```rust
let keep = ctx.register::<u32>("port", 3132);
drop(keep);   // 键消失
```

这不是"为了整洁"的装饰。它带来三个具体要求，后面所有设计都围绕它们：

- 插件的 `mount` 必须返回 `Vec<Disposer>`（它做的每件事都能撤销）；
- `unmount` 逆序撤销（后注册的先撤，与嵌套资源的直觉一致）；
- 崩溃/超时/替换都走同一条撤销路径，没有第二条"清理代码"。

## 三、`EventBus`：四种分发模式

| 模式 | 语义 | 典型用途 |
|---|---|---|
| `Observe` | 扇出，返回值被忽略 | 通知、日志、投影 |
| `Waterfall` | 中间件链，`next()` 交给下一个 | 可改写/可短路的请求 |
| `Parallel` | 所有监听器并发 | 独立副作用 |
| `Serial` | 顺序执行 | 需要确定顺序的审计 |

事件类型通过 `Event` trait 扩展——**新增事件类型 = 新增一个实现**，不改总线。

## 四、`Plugin` + `PluginRegistry`：清单与生命周期

插件 = manifest + 一个挂载闭包。注册表按 id 存它们，并负责：

```
register → mount → (挂载中) → unmount → 可再 mount
```

挂载前会做三件事，顺序是有意的：

1. **依赖校验**（`inject.required` 逐个查 `Ctx`）——缺失就拒绝挂载，**闭包一次都不执行**；
2. **配置注入**（把该插件自己的配置放进 `MountContext.config`）；
3. **树把手注入**（让插件能在自己的 `mount` 里建子作用域）。

## 五、`PluginScope`：插件树

一层作用域 = 一个 `Ctx` + 一个 `PluginRegistry`。父链继承、子遮蔽父、子注册
对父不可见；卸载父插件时它名下的子作用域递归卸载（子先于父）。

这就是 Cordis `ctx.plugin()` 的对应物，但**归属靠挂载期压栈记录**，不靠插件自报：
插件在 `mount` 里 `mctx.nest()` 出来的作用域，自动记在"当前正在挂载的插件"名下。

## 五个概念怎么咬在一起

```prodoc-flow
graph TB
  Ctx[Ctx 服务定位器|/model.md] --> Plugin[Plugin / Registry|/semantics.md]
  Disposer[Disposer 可撤销|/model.md] --> Plugin
  Plugin --> Scope[PluginScope 插件树|/semantics.md]
  Scope --> Ctx
  Bus[EventBus 事件总线|/model.md] --> Plugin
  Plugin --> Transport[运输层 inproc / stdio|/transport.md]
  Transport --> Bus
```

一句话读法：**注册表用 `Disposer` 保证可撤销，用 `Ctx` 表达依赖，用 `Scope`
表达归属，用 `EventBus` 表达通知，用运输层决定这些发生在进程内还是进程外。**
