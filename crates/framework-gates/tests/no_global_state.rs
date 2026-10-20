//! 框架仓自守门禁 #2：**零全局态**。
//!
//! 宿主侧（EchoAgentCore）有 `no_global_state.rs` 扫自己的 `source/` 全树，
//! 基线是 0。框架 crate 迁出后不再被那套门禁覆盖——保护随迁移消失，所以本仓
//! 自己守：框架里出现 `static mut` / `OnceLock` / `LazyLock` 这类进程级单例，
//! 意味着「同一进程里第二套宿主」跑不起来，这直接违背可复用性。
//!
//! 允许的例外：`const`（编译期常量，无状态）与 `const fn`。
//! 测试模块不计（`#[cfg(test)]` 之后不扫）。

use std::path::{Path, PathBuf};

/// 白名单：**空**（v0.3.0 起）。
///
/// 曾经豁免 `crates/echo-context/src/kernel.rs`（进程级引导单元）。它已搬到
/// 宿主侧（EchoAgentCore 的 `echo-bootstrap`）：那是宿主应用的机制——使用者
/// 全在应用层，框架一处不用；而框架导出进程级单例表会直接破坏"同一进程能起
/// 两套宿主"。
///
/// 因此本仓的「零全局态」现在是**无条件**成立的。这个常量保留为空是为了让
/// 将来若真有豁免有显式位置——空着比删掉更难被误用。
const WHITELIST: &[&str] = &[];

/// 会被判为全局态的模式。
const GLOBAL_STATE_PATTERNS: &[&str] = &[
    "OnceLock",
    "LazyLock",
    "lazy_static",
    "thread_local",
    "static mut",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // `bin/` 是可执行夹具，不是库代码。
            if path.file_name().is_some_and(|n| n == "bin") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// 生产代码 = `#[cfg(test)]` 之前的部分。
fn production_part(text: &str) -> &str {
    match text.find("#[cfg(test)]") {
        Some(idx) => &text[..idx],
        None => text,
    }
}

/// 去掉行注释后再判断，避免「文档里提到 OnceLock」被误判。
fn code_part(line: &str) -> &str {
    match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

#[test]
fn framework_libraries_have_no_global_state() {
    let root = repo_root();
    let mut violations = Vec::new();

    let crates_dir = root.join("crates");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates_dir)
        .into_iter()
        .flatten()
        .flatten()
    {
        let src = entry.path().join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
    }
    assert!(!files.is_empty(), "没扫到任何源文件——门禁形同虚设");

    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (idx, line) in production_part(&text).lines().enumerate() {
            let code = code_part(line);
            // `static NAME: T = …` 也算（可变/惰性初始化之外，静态单例同样是
            // 进程级状态；本仓目前一处都没有，保持零基线）。
            let has_static_item = code.trim_start().starts_with("static ");
            let hit = GLOBAL_STATE_PATTERNS.iter().find(|p| code.contains(*p));
            if !has_static_item && hit.is_none() {
                continue;
            }
            let rel = file
                .strip_prefix(&root)
                .unwrap_or(&file)
                .to_string_lossy()
                .to_string();
            if WHITELIST.contains(&rel.as_str()) {
                continue;
            }
            violations.push(format!("{rel}:{}: {}", idx + 1, line.trim()));
        }
    }

    assert!(
        violations.is_empty(),
        "框架层出现了全局态（可复用性要求：同一进程能起多套宿主；白名单应为空）：\n{}",
        violations.join("\n")
    );
}

/// 自检：白名单必须是空的（v0.3.0 起无条件零全局态）。
#[test]
fn whitelist_stays_empty() {
    assert!(
        WHITELIST.is_empty(),
        "框架仓不允许任何全局态豁免。若确有例外，请先论证它与\
         「同一进程能起两套宿主」不冲突；当前多余条目：{WHITELIST:?}"
    );
}

/// 自检：确认扫描逻辑不会把 `const` 当全局态，也不会漏掉真违规。
#[test]
fn classifier_is_sound() {
    assert!(!GLOBAL_STATE_PATTERNS
        .iter()
        .any(|p| "pub const MAX: usize = 16;".contains(p)));
    assert!(GLOBAL_STATE_PATTERNS
        .iter()
        .any(|p| "static CELL: OnceLock<u8> = OnceLock::new();".contains(p)));
    // 注释里提到不该算
    assert!(code_part("// uses OnceLock under the hood").is_empty());
}
