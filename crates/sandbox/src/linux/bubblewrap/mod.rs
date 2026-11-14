// Derived from Codex linux-sandbox/src/bwrap.rs and linux_run_main.rs at
// 57a808a392 (Apache-2.0). Mount isolation with an inner seccomp helper;
// no managed proxy or host IPC mounts.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;

use crate::helper::HelperRequest;

mod bundled;

const PLATFORM_ROOTS: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib64",
    "/etc",
    "/nix/store",
    "/run/current-system/sw",
];

pub(crate) fn run(request: HelperRequest) -> String {
    let Some(helper) = crate::helper::registered_helper()
        .cloned()
        .or_else(|| std::env::current_exe().ok())
    else {
        return "the sandbox helper path is unavailable".into();
    };
    let args = match arguments(&helper, &request) {
        Ok(args) => args,
        Err(error) => return error,
    };
    bundled::run(args)
}

fn arguments(helper: &Path, request: &HelperRequest) -> Result<Vec<OsString>, String> {
    let mut args: Vec<OsString> = [
        "--unshare-user",
        "--unshare-pid",
        "--die-with-parent",
        "--cap-drop",
        "ALL",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    if !request.network_access {
        args.push("--unshare-net".into());
    }
    match &request.readable_roots {
        None => args.extend(["--ro-bind", "/", "/"].map(Into::into)),
        Some(roots) => {
            args.extend(["--tmpfs", "/"].map(Into::into));
            for path in PLATFORM_ROOTS
                .iter()
                .map(Path::new)
                .chain(roots.iter().map(PathBuf::as_path))
            {
                if path.exists() {
                    bind(&mut args, "--ro-bind", path)?;
                }
            }
            bind(&mut args, "--ro-bind", helper)?;
        }
    }
    // Ancestors first: a later /tmp bind must not hide a workspace bind under it.
    let mut roots = crate::roots::resolve_roots(&request.writable_roots);
    roots.sort_by_key(|root| root.root.native().components().count());
    for root in &roots {
        bind(&mut args, "--bind", root.root.native())?;
    }
    // Apply exclusions after every writable mount, including overlapping roots.
    for root in &roots {
        for path in &root.read_only {
            bind(&mut args, "--ro-bind", path.native())?;
        }
    }
    // Codex's no-proc fallback preserves container procfs masks. The explicit
    // user namespace prevents reaching host process root/cwd/fd links.
    args.extend(["--ro-bind", "/proc", "/proc", "--dev", "/dev", "--"].map(Into::into));
    args.push(helper.as_os_str().to_owned());
    args.push(crate::HELPER_ARG.into());
    args.push("--network-only".into());
    args.extend(
        request
            .to_args()
            .ok_or("sandbox arguments must be UTF-8")?
            .into_iter()
            .map(Into::into),
    );
    Ok(args)
}

fn bind(args: &mut Vec<OsString>, flag: &str, path: &Path) -> Result<(), String> {
    let resolved = ResolvedPath::resolve(path).map_err(|e| e.to_string())?;
    args.push(flag.into());
    args.push(resolved.native().as_os_str().to_owned());
    args.push(path.as_os_str().to_owned());
    Ok(())
}

#[cfg(test)]
mod tests;
