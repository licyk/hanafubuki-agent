use super::*;

fn kinds(source: &str) -> Vec<(usize, &'static str, String)> {
    entry_items(source)
        .unwrap()
        .into_iter()
        .map(|item| (item.line, item.kind, item.name))
        .collect()
}

#[test]
fn entry_files_may_hold_declarations_reexports_and_docs() {
    let source = r#"
//! Crate docs.
#![forbid(unsafe_code)]

mod private;
pub mod public;
#[cfg(feature = "x")]
mod optional;
#[cfg(test)]
mod tests;

pub use public::{Thing, Other};
use std::sync::Arc;
extern crate alloc;

/// Re-exports only.
pub mod prelude {
    pub use crate::public::Thing;
}
"#;
    assert_eq!(kinds(source), vec![]);
}

#[test]
fn entry_files_may_hold_proc_macro_entry_points() {
    let source = r#"
#[proc_macro_derive(Service, attributes(service))]
pub fn derive_service(input: TokenStream) -> TokenStream { service::expand(input) }
#[proc_macro_attribute]
pub fn plugin(args: TokenStream, item: TokenStream) -> TokenStream { plugin::expand(args, item) }
"#;
    assert_eq!(kinds(source), vec![]);
}

#[test]
fn reports_every_kind_of_implementation_with_its_line() {
    let source = r#"mod ok;
pub fn helper() {}
pub struct Config;
enum State { A }
pub trait Ext {}
impl Ext for Config {}
impl Config {}
type Alias = u8;
const LIMIT: usize = 3;
static NAME: &str = "x";
macro_rules! shout { () => {} }
union Bits { a: u8 }
thread_local! {}
"#;
    assert_eq!(
        kinds(source),
        vec![
            (2, "fn", "helper".into()),
            (3, "struct", "Config".into()),
            (4, "enum", "State".into()),
            (5, "trait", "Ext".into()),
            (6, "impl", "Ext for Config".into()),
            (7, "impl", "Config".into()),
            (8, "type", "Alias".into()),
            (9, "const", "LIMIT".into()),
            (10, "static", "NAME".into()),
            (11, "macro_rules!", "shout".into()),
            (12, "union", "Bits".into()),
            (13, "macro call", "thread_local".into()),
        ]
    );
}

#[test]
fn checks_inline_modules_including_tests() {
    let source = r#"
pub mod prelude {
    pub use crate::Thing;
    pub fn sneaky() {}
}
#[cfg(test)]
mod tests {
    #[test]
    fn works() {}
}
"#;
    assert_eq!(
        kinds(source),
        vec![(4, "fn", "sneaky".into()), (9, "fn", "works".into())]
    );
}

#[test]
fn a_well_laid_out_workspace_passes() {
    let root = scratch("clean");
    write(&root, "xtask/src/main.rs", "mod task;\nfn main() {}\n");
    write(
        &root,
        "xtask/src/task.rs",
        "pub fn run() {}\n#[cfg(test)]\nmod tests;\n",
    );
    write(
        &root,
        "xtask/src/task/tests.rs",
        "use super::*;\n#[test]\nfn runs() { run() }\n",
    );
    write(&root, "crates/core/Cargo.toml", "");
    write(
        &root,
        "crates/core/src/lib.rs",
        "mod parts;\npub use parts::Part;\n",
    );
    write(
        &root,
        "crates/core/src/parts/mod.rs",
        "mod part;\npub use part::Part;\n",
    );
    write(&root, "crates/core/src/parts/part.rs", "pub struct Part;\n");
    write(
        &root,
        "crates/core/tests/single.rs",
        "#[test]\nfn single() {}\n",
    );
    write(
        &root,
        "crates/core/tests/suite/main.rs",
        "#[path = \"../support/mod.rs\"]\nmod support;\n",
    );
    write(
        &root,
        "crates/core/tests/support/mod.rs",
        "pub fn helper() {}\n",
    );
    assert_eq!(violations(&root).unwrap(), Vec::<String>::new());
}

#[test]
fn reports_each_layout_rule() {
    let root = scratch("violations");
    write(&root, "crates/core/Cargo.toml", "");
    write(
        &root,
        "crates/core/src/lib.rs",
        "mod a;\nmod b;\nmod c;\npub struct Inline;\n",
    );
    // production children beside a.rs
    write(&root, "crates/core/src/a.rs", "mod child;\n");
    write(&root, "crates/core/src/a/child.rs", "pub fn child() {}\n");
    // only tests below b/mod.rs, and an implementation in it
    write(
        &root,
        "crates/core/src/b/mod.rs",
        "pub fn b() {}\n#[cfg(test)]\nmod tests;\n",
    );
    write(&root, "crates/core/src/b/tests.rs", "");
    write(
        &root,
        "crates/core/src/c.rs",
        "#[path = \"x.rs\"]\nmod x;\npub use x::*;\n#[test]\nfn inline() {}\n",
    );
    write(&root, "crates/core/tests/suite.rs", "");
    write(&root, "crates/core/tests/suite/case.rs", "");
    let violations = violations(&root).unwrap();
    let starts: Vec<&str> = violations
        .iter()
        .map(|violation| violation.split(". ").next().unwrap())
        .collect();
    assert_eq!(
        starts,
        [
            "crates/core/src/a.rs: module with production children",
            "crates/core/src/b/mod.rs: module whose only children are tests",
            "crates/core/src/c.rs:1: `#[path]` in src/",
            "crates/core/src/c.rs:3: glob re-export",
            "crates/core/src/c.rs:4: inline test in a production file",
            "crates/core/src/lib.rs:4: struct `Inline` in lib.rs",
            "crates/core/tests/suite.rs: multi-file suite rooted beside its directory",
            "crates/core/tests/suite: test directory with no main.rs",
        ]
    );
}

#[test]
fn a_workspace_without_crates_checks_only_xtask() {
    let root = scratch("empty");
    write(&root, "xtask/src/main.rs", "fn main() {}\n");
    assert_eq!(package_roots(&root), [root.join("xtask")]);
    assert!(violations(&root).unwrap().is_empty());
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xtask-layout-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(root: &Path, path: &str, contents: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}
