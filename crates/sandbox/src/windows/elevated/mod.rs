//! The elevated Windows sandbox: UAC provisions two restricted identities;
//! normal command launches use the offline identity unless networking is allowed.

mod credentials;
mod firewall;
mod launch;
mod pipes;
mod security;
mod setup;

pub(crate) use launch::{run, RUNNER_ARG};
pub(crate) use setup::{ready, setup, SETUP_ARG};

pub(crate) fn dispatch(
    first: &std::ffi::OsStr,
    mut args: impl Iterator<Item = std::ffi::OsString>,
) -> Option<Result<u32, String>> {
    if first == SETUP_ARG {
        return Some(
            args.next()
                .ok_or_else(|| "sandbox setup needs its directory".to_string())
                .and_then(|path| setup::elevated_setup(path.into()))
                .map(|()| 0),
        );
    }
    if first != RUNNER_ARG {
        return None;
    }
    Some((|| {
        let name = args
            .next()
            .and_then(|name| name.into_string().ok())
            .ok_or("sandbox runner needs its IO endpoint")?;
        // Keep standard handles alive through helper_dispatch error reporting;
        // this dedicated runner exits immediately after dispatch returns.
        let _stdio = std::mem::ManuallyDrop::new(pipes::attach(&name)?);
        let request = crate::helper::HelperRequest::parse(args.collect())?;
        super::run_prepared(request)
    })())
}
