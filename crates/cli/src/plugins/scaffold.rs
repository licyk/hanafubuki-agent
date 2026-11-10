//! `plugins new`: a plugin with code, from the templates under
//! `templates/plugins/` (Rust as a process or a WebAssembly module, Python
//! as a process). The plugin the Agent installs is the directory's
//! `package/`; around it, the guest program's source and how to build it.
//! `examples/plugins/` holds these templates' output.

use std::path::{Path, PathBuf};

use crate::cli::{PluginLanguage, PluginTarget};
use crate::exit::Failure;

/// The files a language's plugin starts with: where each goes, and its
/// template, in which `{{name}}` is the plugin's name and `{{kind}}` that
/// name as a context kind (and a Cargo library's file name).
fn templates(
    language: PluginLanguage,
    target: PluginTarget,
) -> &'static [(&'static str, &'static str)] {
    match (language, target) {
        (PluginLanguage::Rust, PluginTarget::Wasm) => &[
            (
                "Cargo.toml",
                include_str!("../../templates/plugins/rust-wasm/Cargo.toml.tmpl"),
            ),
            (
                "src/lib.rs",
                include_str!("../../templates/plugins/rust-wasm/lib.rs.tmpl"),
            ),
            (
                "package/.hanafubuki-plugin/plugin.json",
                include_str!("../../templates/plugins/rust-wasm/plugin.json.tmpl"),
            ),
            (
                "README.md",
                include_str!("../../templates/plugins/rust-wasm/README.md.tmpl"),
            ),
            (
                ".gitignore",
                include_str!("../../templates/plugins/rust-wasm/gitignore.tmpl"),
            ),
        ],
        (PluginLanguage::Rust, PluginTarget::Process) => &[
            (
                "Cargo.toml",
                include_str!("../../templates/plugins/rust/Cargo.toml.tmpl"),
            ),
            (
                "src/main.rs",
                include_str!("../../templates/plugins/rust/main.rs.tmpl"),
            ),
            (
                "package/.hanafubuki-plugin/plugin.json",
                include_str!("../../templates/plugins/rust/plugin.json.tmpl"),
            ),
            (
                "README.md",
                include_str!("../../templates/plugins/rust/README.md.tmpl"),
            ),
            (
                ".gitignore",
                include_str!("../../templates/plugins/rust/gitignore.tmpl"),
            ),
        ],
        (PluginLanguage::Python, _) => &[
            (
                "plugin.py",
                include_str!("../../templates/plugins/python/plugin.py.tmpl"),
            ),
            (
                "package/.hanafubuki-plugin/plugin.json",
                include_str!("../../templates/plugins/python/plugin.json.tmpl"),
            ),
            (
                "README.md",
                include_str!("../../templates/plugins/python/README.md.tmpl"),
            ),
            (
                ".gitignore",
                include_str!("../../templates/plugins/python/gitignore.tmpl"),
            ),
        ],
    }
}

pub(crate) fn run(
    name: &str,
    language: PluginLanguage,
    target: PluginTarget,
    dir: Option<PathBuf>,
) -> Result<(), Failure> {
    check_name(name)?;
    if (language, target) == (PluginLanguage::Python, PluginTarget::Wasm) {
        return Err(Failure::usage(
            "`--target wasm` writes a Rust plugin: a Python guest runs as a process",
        ));
    }
    let dir = dir.unwrap_or_else(|| PathBuf::from(name));
    let occupied = std::fs::read_dir(&dir).is_ok_and(|mut entries| entries.next().is_some());
    if occupied {
        return Err(Failure::new(format!(
            "{} already exists and is not empty",
            dir.display()
        )));
    }
    let kind = name.replace('-', "_");
    for (path, template) in templates(language, target) {
        let contents = template
            .replace("{{name}}", name)
            .replace("{{kind}}", &kind);
        write(&dir.join(path), &contents)?;
    }
    println!("Created plugin `{name}` in {}.", dir.display());
    let build = match (language, target) {
        (PluginLanguage::Rust, PluginTarget::Wasm) => format!(
            "cargo build --release --target wasm32-unknown-unknown && \
             cp target/wasm32-unknown-unknown/release/{kind}.wasm package/{name}.wasm"
        ),
        (PluginLanguage::Rust, PluginTarget::Process) => {
            format!("cargo build --release && cp target/release/{name} package/")
        }
        (PluginLanguage::Python, _) => {
            format!("python3 -m pykardia.guest.export process plugin.py Plugin package/{name}.pyz")
        }
    };
    let built = match target {
        PluginTarget::Process => "program",
        PluginTarget::Wasm => "module",
    };
    println!("Build its {built} into the package, in {}:", dir.display());
    println!("  {build}");
    println!("Then run it in the Agent, reloaded at each build:");
    println!(
        "  hanafubuki-agent plugins dev {}",
        dir.join("package").display()
    );
    Ok(())
}

/// A name every place it goes takes as it is: a plugin's, a Cargo
/// package's, a program's, and (with `_` for `-`) a context kind's.
fn check_name(name: &str) -> Result<(), Failure> {
    let mut characters = name.chars();
    let valid = characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && name.len() <= 48;
    if !valid {
        return Err(Failure::usage(format!(
            "`{name}` is not a plugin name for `plugins new`: lowercase letters, digits and `-`, \
             starting with a letter, at most 48 characters"
        )));
    }
    Ok(())
}

fn write(path: &Path, contents: &str) -> Result<(), Failure> {
    let failed = |error: std::io::Error| Failure::new(format!("{}: {error}", path.display()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    std::fs::write(path, contents).map_err(failed)
}
