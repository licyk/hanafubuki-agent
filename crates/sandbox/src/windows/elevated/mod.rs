//! The elevated Windows sandbox: UAC provisions two restricted identities;
//! normal command launches use the offline identity unless networking is allowed.

mod credentials;
mod firewall;
mod launch;
mod pipes;
mod security;
mod setup;

pub(super) use security::current_user_sid;

pub(crate) use launch::{run, RUNNER_ARG};
pub(crate) use setup::{ready, setup, SETUP_ARG};

pub(crate) fn dispatch(
    first: &std::ffi::OsStr,
    mut args: impl Iterator<Item = std::ffi::OsString>,
) -> Option<Result<u32, String>> {
    if first == SETUP_ARG {
        return Some((|| {
            let root = args.next().ok_or("sandbox setup needs its directory")?;
            let reader = args
                .next()
                .and_then(|sid| sid.into_string().ok())
                .ok_or("sandbox setup needs the requesting user's SID")?;
            let reader = super::token::LocalSid::from_string(&reader)?;
            setup::elevated_setup(root.into(), reader)?;
            Ok(0)
        })());
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
        super::run_prepared(request, true)
    })())
}
