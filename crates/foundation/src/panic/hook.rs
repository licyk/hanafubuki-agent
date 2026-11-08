//! The application's panic hook: report every panic, let a contained one
//! unwind to its catch, abort on any other.

use std::backtrace::Backtrace;
use std::cell::Cell;
use std::io::Write;
use std::panic::PanicHookInfo;

use super::containment::is_panic_contained;
use super::crash_record::write_crash_record;
use super::faults::{record_panic, PanicFault};
use super::panic_message;

thread_local! {
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

/// Installs the process panic hook, replacing the default one. Call it once,
/// first thing in the application entrypoint; tests do not install it, so a
/// failing test still unwinds to the harness.
///
/// On every panic the hook writes the thread, location, message and a
/// backtrace to stderr, then records the panic ([`super::recorded_panics`]).
/// A contained panic ([`super::is_panic_contained`]) is logged at error and
/// unwinds to its catch. Any other is written to the crash record (when the
/// application named one, [`super::configure_crash_record`]), logged at
/// error, the logger is flushed and the process aborts, as it did when
/// release builds aborted on panic.
///
/// `logging_blocked` says whether the current thread is inside the logger
/// now (a log record being written): the hook then leaves `log` alone,
/// since the logger's own locks are held, and stderr carries the report.
pub fn install_panic_hook(logging_blocked: fn() -> bool) {
    std::panic::set_hook(Box::new(move |info| report_panic(info, logging_blocked)));
}

fn report_panic(info: &PanicHookInfo<'_>, logging_blocked: fn() -> bool) {
    let reentered = IN_HOOK
        .try_with(|inside| inside.replace(true))
        .unwrap_or(true);
    let contained = is_panic_contained();
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("<unnamed>").to_string();
    let location = info
        .location()
        .map(ToString::to_string)
        .unwrap_or_else(|| "<unknown location>".to_string());
    let message = panic_message(info.payload());
    if reentered {
        // Reporting the first panic panicked: say so where it cannot fail
        // again, and stop.
        let _ = writeln!(
            std::io::stderr(),
            "thread '{thread_name}' panicked at {location} while reporting a panic: {message}; aborting"
        );
        std::process::abort();
    }
    let backtrace = Backtrace::force_capture();
    let outcome = if contained {
        "contained: unwinding to its catch"
    } else {
        "fatal: aborting"
    };
    let _ = writeln!(
        std::io::stderr(),
        "thread '{thread_name}' panicked at {location}:\n{message}\n({outcome})\nstack backtrace:\n{backtrace}"
    );
    let fault = PanicFault::now(
        thread_name.clone(),
        location.clone(),
        message.clone(),
        contained,
    );
    if !contained {
        // The process is about to abort: leave the next run a record.
        write_crash_record(&fault);
    }
    record_panic(fault);
    let may_log = !logging_blocked();
    if contained {
        if may_log {
            log::error!(
                "contained panic on thread '{thread_name}' at {location}: {message}\nstack backtrace:\n{backtrace}"
            );
        }
        let _ = IN_HOOK.try_with(|inside| inside.set(false));
        return;
    }
    if may_log {
        log::error!(
            "fatal panic on thread '{thread_name}' at {location}: {message}; aborting\nstack backtrace:\n{backtrace}"
        );
        log::logger().flush();
    }
    std::process::abort();
}
