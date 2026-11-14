// Derived from Codex codex-rs/bwrap/src/main.rs at 57a808a392 (Apache-2.0).

use std::ffi::{CString, OsString};
use std::os::unix::ffi::OsStrExt;

/// Called only by the single-threaded helper, before the Agent starts.
pub(super) fn run(args: Vec<OsString>) -> String {
    unsafe extern "C" {
        fn hanafubuki_bwrap_main(
            argc: libc::c_int,
            argv: *const *const libc::c_char,
        ) -> libc::c_int;
    }
    let args = std::iter::once(OsString::from("bwrap"))
        .chain(args)
        .map(|arg| CString::new(arg.as_bytes()))
        .collect::<Result<Vec<_>, _>>();
    let args = match args {
        Ok(args) => args,
        Err(_) => return "sandbox argument contains NUL".into(),
    };
    let mut pointers: Vec<_> = args.iter().map(|arg| arg.as_ptr()).collect();
    pointers.push(std::ptr::null());
    // SAFETY: argv is NUL-terminated, its strings stay alive, and the helper is
    // single-threaded. bwrap owns its namespace processes until they terminate.
    let code = unsafe { hanafubuki_bwrap_main(args.len() as libc::c_int, pointers.as_ptr()) };
    std::process::exit(code)
}
