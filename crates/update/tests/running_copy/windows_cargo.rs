//! Windows: whether `cargo install` can replace an executable that is
//! running (the plan's correction 10), which decides whether a cargo
//! installation is updated by cargo there too. A probe crate without
//! dependencies is installed offline into a temporary root, started, and
//! installed again over itself while it runs; the outcome is printed for
//! the record. It is an observation: a probe that cannot be set up is
//! reported and skipped.

use std::io;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const MANIFEST: &str = "[package]\nname = \"running-copy-probe\"\nversion = \"0.1.0\"\n\
                        edition = \"2021\"\n\n[workspace]\n";

pub(crate) fn cargo_install_over_a_running_copy(root: &Path) {
    let probe = root.join("probe");
    let installed = root.join("probe-root");
    std::fs::create_dir_all(probe.join("src")).unwrap();
    std::fs::write(probe.join("Cargo.toml"), MANIFEST).unwrap();
    write_main(&probe, "first");
    match install(root, &probe, &installed) {
        Ok(first) if first.status.success() => {}
        Ok(first) => return skipped(&super::describe(&first)),
        Err(error) => return skipped(&error.to_string()),
    }
    let executable = installed.join("bin").join(format!(
        "running-copy-probe{}",
        std::env::consts::EXE_SUFFIX
    ));
    let mut running = match Command::new(&executable)
        .arg("wait")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(running) => running,
        Err(error) => return skipped(&error.to_string()),
    };
    write_main(&probe, "second");
    let second = install(root, &probe, &installed);
    let reported = Command::new(&executable)
        .output()
        .map(|now| super::stdout(&now).trim().to_string())
        .unwrap_or_else(|error| error.to_string());
    let outcome = match &second {
        Ok(second) => {
            let stderr = String::from_utf8_lossy(&second.stderr);
            let last_line = stderr.lines().last().unwrap_or_default().to_string();
            let result = if second.status.success() {
                "succeeded"
            } else {
                "failed"
            };
            format!("{result} ({last_line})")
        }
        Err(error) => format!("did not start ({error})"),
    };
    eprintln!(
        "running copy: cargo install over a running copy: {outcome}; the installed file now \
         prints {reported:?}"
    );
    assert_eq!(
        super::finish(&mut running),
        "first",
        "the running probe went on"
    );
}

fn skipped(reason: &str) {
    eprintln!("running copy: cargo install over a running copy: not tried ({reason})");
}

/// A `main` that prints `name`, and with `wait` first waits for its input
/// to end.
fn write_main(probe: &Path, name: &str) {
    std::fs::write(
        probe.join("src").join("main.rs"),
        format!(
            "fn main() {{\n    if std::env::args().nth(1).as_deref() == Some(\"wait\") {{\n        \
             let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());\n    }}\n    \
             println!(\"{name}\");\n}}\n"
        ),
    )
    .unwrap();
}

/// `cargo install --path probe --root installed --offline --force`, by the
/// cargo running this test, its home and target directory temporary.
fn install(root: &Path, probe: &Path, installed: &Path) -> io::Result<Output> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    Command::new(cargo)
        .args(["install", "--offline", "--force", "--path"])
        .arg(probe)
        .arg("--root")
        .arg(installed)
        .env("CARGO_HOME", root.join("cargo-home"))
        .env("CARGO_TARGET_DIR", root.join("probe-target"))
        .current_dir(root)
        .output()
}
