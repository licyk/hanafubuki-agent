//! The guest host end to end, with real guest processes: this binary is
//! also the guest (kardia-guest's `serve`) and the sandbox helper, so the
//! guests it starts, and confines, re-execute it; and with WASM guests
//! written in the text format. Without a harness, `main`
//! answers those command lines first, then runs each check in turn (those
//! whose name contains one of the arguments, when there are any).

mod contribute;
mod faults;
mod fixture;
mod guest;
mod host_calls;
mod lifecycle;
mod logging;
mod sandbox;
mod wasm;

use std::future::Future;
use std::pin::Pin;

type Check = fn() -> Pin<Box<dyn Future<Output = ()>>>;

const CHECKS: &[(&str, Check)] = &[
    ("a_guest_starts_is_disabled_enabled_and_uninstalled", || {
        Box::pin(lifecycle::a_guest_starts_is_disabled_enabled_and_uninstalled())
    }),
    (
        "a_guest_without_its_permissions_starts_only_once_they_are_granted",
        || Box::pin(lifecycle::a_guest_without_its_permissions_starts_only_once_they_are_granted()),
    ),
    (
        "a_guest_gets_only_its_environment_and_runs_in_its_data_directory",
        || Box::pin(lifecycle::a_guest_gets_only_its_environment_and_runs_in_its_data_directory()),
    ),
    (
        "a_guest_process_is_the_instances_and_stops_with_the_guest_host",
        || Box::pin(lifecycle::a_guest_process_is_the_instances_and_stops_with_the_guest_host()),
    ),
    (
        "a_killed_guest_fails_and_the_restart_loop_brings_it_back",
        || Box::pin(faults::a_killed_guest_fails_and_the_restart_loop_brings_it_back()),
    ),
    (
        "a_call_past_its_deadline_kills_the_guest_which_restarts",
        || Box::pin(faults::a_call_past_its_deadline_kills_the_guest_which_restarts()),
    ),
    (
        "a_fourth_fault_in_the_window_leaves_the_guest_failed_until_it_is_retried",
        || {
            Box::pin(
                faults::a_fourth_fault_in_the_window_leaves_the_guest_failed_until_it_is_retried(),
            )
        },
    ),
    ("a_changed_package_does_not_start", || {
        Box::pin(faults::a_changed_package_does_not_start())
    }),
    ("a_start_that_hangs_or_fails_stays_failed", || {
        Box::pin(faults::a_start_that_hangs_or_fails_stays_failed())
    }),
    ("a_guests_own_words_are_logged_only_at_trace", || {
        Box::pin(logging::a_guests_own_words_are_logged_only_at_trace())
    }),
    ("a_guest_contributes_what_its_manifest_declares", || {
        Box::pin(contribute::a_guest_contributes_what_its_manifest_declares())
    }),
    (
        "a_guests_contributions_retire_with_it_and_its_failures_stay_its_own",
        || {
            Box::pin(
                contribute::a_guests_contributions_retire_with_it_and_its_failures_stay_its_own(),
            )
        },
    ),
    ("a_guest_views_only_its_workspace_and_its_settings", || {
        Box::pin(host_calls::a_guest_views_only_its_workspace_and_its_settings())
    }),
    ("a_guest_that_provides_another_service_is_stopped", || {
        Box::pin(host_calls::a_guest_that_provides_another_service_is_stopped())
    }),
    (
        "the_sandbox_keeps_a_guest_to_its_package_its_data_and_its_grant",
        || Box::pin(sandbox::the_sandbox_keeps_a_guest_to_its_package_its_data_and_its_grant()),
    ),
    (
        "a_wasm_guest_runs_without_a_process_and_contributes_a_tool",
        || Box::pin(wasm::a_wasm_guest_runs_without_a_process_and_contributes_a_tool()),
    ),
    (
        "an_endless_wasm_call_is_stopped_and_the_guest_restarts_alone",
        || Box::pin(wasm::an_endless_wasm_call_is_stopped_and_the_guest_restarts_alone()),
    ),
    (
        "a_wasm_guest_that_outgrows_its_memory_traps_and_restarts",
        || Box::pin(wasm::a_wasm_guest_that_outgrows_its_memory_traps_and_restarts()),
    ),
    (
        "a_trapping_wasm_guests_contributions_retire_and_return",
        || Box::pin(wasm::a_trapping_wasm_guests_contributions_retire_and_return()),
    ),
];

fn main() {
    hanafubuki_agent_sandbox::helper_dispatch();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some(guest::GUEST_ARG) {
        guest::serve(&args[1..]);
        return;
    }
    logging::capture();
    let filters: Vec<&String> = args.iter().filter(|arg| !arg.starts_with('-')).collect();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    for (name, check) in CHECKS {
        if !filters.is_empty() && !filters.iter().any(|filter| name.contains(filter.as_str())) {
            continue;
        }
        runtime.block_on(check());
        eprintln!("ok: {name}");
    }
}
