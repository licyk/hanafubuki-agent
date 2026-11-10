//! The test guest: this binary, started with [`GUEST_ARG`] and its plugin's
//! name, serves the guest protocol with kardia-guest. Its context section
//! names the plugin, and its tools fail as their names say: `echo` answers,
//! `panic` panics (kardia-guest makes that the call's error), `hang` never
//! answers, `exit` exits, `abort` aborts, `segfault` raises `SIGSEGV`,
//! `oom` runs out of the address space it limits itself to, `garbage`
//! writes a line that is not the protocol to stdout, and `huge` answers
//! with more than the frame limit. With the mode `hang-context`, its
//! context section never answers.

use std::sync::OnceLock;
use std::time::Duration;

use kardia_guest::{json, Guest, Host, Map, Meta, Request, Result, Value};

/// The first argument that makes this binary the test guest.
pub const GUEST_ARG: &str = "--hanafubuki-crash-guest";

/// Every tool the guest declares.
pub const TOOLS: &[&str] = &[
    "echo", "panic", "hang", "exit", "abort", "segfault", "oom", "garbage", "huge",
];

static NAME: OnceLock<String> = OnceLock::new();
static MODE: OnceLock<String> = OnceLock::new();

/// Serves the guest named by `args[0]`, in the mode `args[1]`.
pub fn serve(args: &[String]) {
    let _ = NAME.set(args.first().cloned().unwrap_or_default());
    let _ = MODE.set(args.get(1).cloned().unwrap_or_default());
    no_core_dumps();
    kardia_guest::serve::<CrashGuest>();
}

/// Its crashes leave no core files behind.
#[cfg(unix)]
fn no_core_dumps() {
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: setting this process's own limit from a valid struct.
    unsafe {
        libc::setrlimit(libc::RLIMIT_CORE, &none);
    }
}

#[cfg(not(unix))]
fn no_core_dumps() {}

struct CrashGuest;

fn name() -> &'static str {
    NAME.get().map(String::as_str).unwrap_or_default()
}

fn hangs() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

impl Guest for CrashGuest {
    type Config = Value;

    fn meta() -> Meta {
        Meta::plugin(name()).provide("plugin")
    }

    fn apply(host: &Host, _config: Value, _imports: Map<String, Value>) -> Result<Self> {
        host.provide("plugin", &json!({}))?;
        Ok(CrashGuest)
    }

    fn call(&mut self, _host: &Host, request: Request) -> Result<Value> {
        let args = &request.args;
        match request.method.as_str() {
            "tool" => tool(
                args["name"].as_str().unwrap_or_default(),
                &args["arguments"],
            ),
            "context" if MODE.get().map(String::as_str) == Some("hang-context") => hangs(),
            "context" => Ok(json!(format!("The {} board is open.", name()))),
            method => Err(format!("no method {method}").into()),
        }
    }
}

fn tool(tool: &str, arguments: &Value) -> Result<Value> {
    match tool {
        "echo" => Ok(json!({ "echoed": arguments, "by": name() })),
        "panic" => panic!("the {} guest panicked on purpose", name()),
        "hang" => hangs(),
        "exit" => std::process::exit(5),
        "abort" => std::process::abort(),
        "segfault" => segfault(),
        "oom" => out_of_memory(),
        "garbage" => {
            use std::io::Write;
            // stdout carries the protocol: this line breaks it.
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(b"this is not the guest protocol\n");
            let _ = stdout.flush();
            hangs()
        }
        // Over the guest host's 4 MiB frame limit.
        "huge" => Ok(json!("x".repeat(5 * 1024 * 1024))),
        tool => Err(format!("no tool {tool}").into()),
    }
}

#[cfg(unix)]
fn segfault() -> ! {
    // Rust's runtime handles SIGSEGV (to report a stack overflow) and
    // returns from a raised one; the default action ends the process.
    // SAFETY: setting this process's own signal disposition, then raising.
    unsafe {
        libc::signal(libc::SIGSEGV, libc::SIG_DFL);
        libc::raise(libc::SIGSEGV);
    }
    unreachable!("SIGSEGV ends the process")
}

#[cfg(not(unix))]
fn segfault() -> ! {
    std::process::abort()
}

/// Limits the address space to 256 MiB, then asks for 1 GiB: the allocation
/// fails and the process aborts, as a guest out of memory does. Nothing is
/// touched, so the machine's memory is never at stake.
#[cfg(unix)]
fn out_of_memory() -> ! {
    let limit = libc::rlimit {
        rlim_cur: 256 << 20,
        rlim_max: 256 << 20,
    };
    // SAFETY: setting this process's own limit from a valid struct.
    unsafe {
        libc::setrlimit(libc::RLIMIT_AS, &limit);
    }
    let reserved: Vec<u8> = Vec::with_capacity(1 << 30);
    std::hint::black_box(reserved);
    unreachable!("the allocation over the limit aborts")
}

#[cfg(not(unix))]
fn out_of_memory() -> ! {
    std::process::abort()
}
