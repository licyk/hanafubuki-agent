//! Every failure of an installed plugin leaves the Agent working, end to
//! end: the whole Agent over a scripted model, with real plugin processes.
//! For each failure (a bad manifest or component, an MCP server's crash, a
//! hook's failure, a guest's panic, hang, abort, segfault, exhausted memory,
//! protocol garbage and oversize frame, a restart loop, and a panic of the
//! `plugins` row itself) a check makes it happen in a turn or beside one,
//! asserts what the plugin's status shows, and then runs a following turn
//! that completes normally with the other plugins' contributions.
//!
//! This binary is also every plugin process the checks install: the guest
//! (kardia-guest's `serve`, with [`guest::GUEST_ARG`]), the stdio MCP server
//! ([`mcp_server::MCP_ARG`]) and the sandbox helper, which confines the
//! guests. Without a harness, `main` answers those command lines first, then
//! runs each check in turn (those whose name contains one of the arguments,
//! when there are any), each under a watchdog. A guest runs only under the
//! sandbox's plugin profile, checked on Linux: elsewhere, or where the
//! sandbox is unavailable, the checks of guests are skipped and the rest
//! run.

mod content;
mod fixture;
mod guest;
mod guests;
mod mcp_server;
mod row;
mod watchdog;

use std::future::Future;
use std::pin::Pin;

type Check = fn() -> Pin<Box<dyn Future<Output = ()>>>;

const CHECKS: &[(&str, Check)] = &[
    ("a_bad_manifest_or_component_costs_only_its_package", || {
        Box::pin(content::a_bad_manifest_or_component_costs_only_its_package())
    }),
    (
        "a_plugins_mcp_server_that_crashes_is_reconnected_and_the_turn_goes_on",
        || {
            Box::pin(
                content::a_plugins_mcp_server_that_crashes_is_reconnected_and_the_turn_goes_on(),
            )
        },
    ),
    ("a_plugins_failing_hooks_have_no_effect", || {
        Box::pin(content::a_plugins_failing_hooks_have_no_effect())
    }),
    ("a_guest_panic_is_the_tool_calls_error", || {
        Box::pin(guests::a_guest_panic_is_the_tool_calls_error())
    }),
    (
        "a_hanging_guest_is_killed_and_the_turn_goes_on_without_it",
        || Box::pin(guests::a_hanging_guest_is_killed_and_the_turn_goes_on_without_it()),
    ),
    (
        "a_guest_that_aborts_segfaults_or_runs_out_of_memory_restarts",
        || Box::pin(guests::a_guest_that_aborts_segfaults_or_runs_out_of_memory_restarts()),
    ),
    (
        "protocol_garbage_or_an_oversize_frame_is_a_guest_crash",
        || Box::pin(guests::protocol_garbage_or_an_oversize_frame_is_a_guest_crash()),
    ),
    (
        "a_restart_loop_leaves_the_guest_failed_until_it_is_retried",
        || Box::pin(guests::a_restart_loop_leaves_the_guest_failed_until_it_is_retried()),
    ),
    (
        "a_panic_of_the_plugins_row_stops_the_guests_and_spares_the_content",
        || Box::pin(row::a_panic_of_the_plugins_row_stops_the_guests_and_spares_the_content()),
    ),
];

fn main() {
    hanafubuki_agent::sandbox::helper_dispatch();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some(guest::GUEST_ARG) => return guest::serve(&args[1..]),
        Some(mcp_server::MCP_ARG) => return mcp_server::serve(&args[1..]),
        _ => {}
    }
    let filters: Vec<String> = args
        .into_iter()
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    // A check's future is the whole Agent's turn, polled by `block_on` on
    // the calling thread: in a debug build it outgrows the 1 MiB main
    // thread Windows gives a program, so the checks run on a thread of
    // their own.
    std::thread::Builder::new()
        .name("checks".into())
        .stack_size(CHECK_STACK)
        .spawn(move || run_checks(&filters))
        .unwrap()
        .join()
        .unwrap();
}

const CHECK_STACK: usize = 32 * 1024 * 1024;

fn run_checks(filters: &[String]) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(CHECK_STACK)
        .build()
        .unwrap();
    for (name, check) in CHECKS {
        if !filters.is_empty() && !filters.iter().any(|filter| name.contains(filter.as_str())) {
            continue;
        }
        watchdog::guard(name, || runtime.block_on(check()));
        eprintln!("ok: {name}");
    }
}
