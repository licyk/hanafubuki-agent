// Derived from Codex codex-rs/bwrap/build.rs at 57a808a392 (Apache-2.0).
// Embed the helper so release archives and cargo installs need no external bwrap.

fn main() {
    println!("cargo:rerun-if-changed=vendor/bubblewrap");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    std::fs::write(
        out.join("config.h"),
        "#pragma once\n#define PACKAGE_STRING \"bubblewrap 0.11.2 for hanafubuki-agent\"\n",
    )
    .expect("write bubblewrap build configuration");
    let cap = pkg_config::Config::new()
        .probe("libcap")
        .expect("Linux sandbox needs libcap development headers and pkg-config");
    let mut build = cc::Build::new();
    for source in ["bubblewrap.c", "bind-mount.c", "network.c", "utils.c"] {
        build.file(format!("vendor/bubblewrap/{source}"));
    }
    build
        .include(&out)
        .include("vendor/bubblewrap")
        .define("_GNU_SOURCE", None)
        .define("main", Some("hanafubuki_bwrap_main"));
    for include in cap.include_paths {
        build.include(include);
    }
    build.compile("hanafubuki_bwrap");
}
