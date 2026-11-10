//! The failures of a code plugin's guest, each in a tool call of a turn
//! beside other guests: a panic is that call's error and the guest runs
//! on; a hang, an abort, a segfault, exhausted memory, a kill, protocol
//! garbage and an oversize frame each end the guest's process, make the
//! call's error, and the restart loop brings it back; a fourth fault in the
//! window leaves it failed until it is retried. The turn ends normally
//! every time, and the following turns use the other guests and the one
//! that came back.

use hanafubuki_agent::{GuestExitCause, GuestState, GuestStatus};

use crate::fixture::{answer, board, call, running, sandboxed, Agent};

/// The model's answer after a failed call of `plugin`'s tool.
fn going_on(plugin: &str) -> String {
    format!("The {plugin} plugin failed; going on.")
}

/// The script of a turn calling `plugin`'s `tool`, then answering.
fn failing_turn(plugin: &str, tool: &str) -> [Vec<u8>; 2] {
    [call(plugin, tool), answer(&going_on(plugin))]
}

/// The script of a turn calling `plugin`'s `echo`, then answering.
fn echo_turn(plugin: &str) -> [Vec<u8>; 2] {
    [call(plugin, "echo"), answer(&format!("{plugin} echoed."))]
}

/// Runs a turn that calls `plugin`'s failing `tool`: it ends normally, and
/// the model read the call's error.
async fn fail_in_a_turn(agent: &Agent, plugin: &str, tool: &str) -> String {
    let outcome = agent.turn(&format!("Call {plugin}'s {tool}.")).await;
    assert_eq!(outcome.final_answer, going_on(plugin));
    let after = agent.last_request();
    assert!(
        after.contains(&format!("the plugin {plugin}@local failed")),
        "{after}"
    );
    after
}

/// Checks `status` is a guest back after its `restarts`th fault, whose last
/// process ended by `cause`, with a status or reason saying `why`.
fn back_after(status: &GuestStatus, restarts: usize, cause: GuestExitCause, why: &str) {
    assert!(running(status), "{status:#?}");
    assert_eq!(status.restarts, restarts, "{status:#?}");
    let exit = status.last_exit.as_ref().unwrap();
    assert_eq!(exit.cause, cause, "{status:#?}");
    let said = format!(
        "{} {}",
        exit.reason.as_deref().unwrap_or_default(),
        exit.status.as_deref().unwrap_or_default()
    );
    assert!(said.contains(why), "{why:?} not in {said:?}");
}

pub async fn a_guest_panic_is_the_tool_calls_error() {
    if !sandboxed() {
        return;
    }
    let script = [
        failing_turn("demo", "panic"),
        echo_turn("other"),
        echo_turn("demo"),
    ];
    let agent = Agent::start("crashes-panic", script.concat()).await;
    let before = agent.install_guest("demo", "run").await;
    agent.install_guest("other", "run").await;

    let after = fail_in_a_turn(&agent, "demo", "panic").await;
    assert!(after.contains("panicked on purpose"), "{after}");
    // The SDK caught it: the same process runs on, with no fault.
    let status = agent.guest("demo@local").await;
    assert!(running(&status), "{status:#?}");
    assert_eq!(status.process_id, before.process_id);
    assert_eq!(status.restarts, 0);

    agent.echo_turn("other", "other echoed.").await;
    let request = agent.last_request();
    assert!(request.contains(&board("demo")), "{request}");
    assert!(request.contains(&board("other")), "{request}");
    agent.echo_turn("demo", "demo echoed.").await;
    agent.finish().await;
}

pub async fn a_hanging_guest_is_killed_and_the_turn_goes_on_without_it() {
    if !sandboxed() {
        return;
    }
    let script = [
        failing_turn("demo", "hang"),
        echo_turn("other"),
        echo_turn("demo"),
    ];
    let agent = Agent::start("crashes-hang", script.concat()).await;
    let before = agent.install_guest("demo", "run").await;
    agent.install_guest("other", "run").await;
    // Its context section never answers.
    agent.install_guest("sleepy", "hang-context").await;

    // The tool's 1 s timeout kills the guest; the section is left out
    // within the plugin call deadline.
    let after = fail_in_a_turn(&agent, "demo", "hang").await;
    assert!(after.contains("did not answer in time"), "{after}");
    let first = agent.request(0);
    assert!(first.contains(&board("demo")), "{first}");
    assert!(first.contains(&board("other")), "{first}");
    assert!(!first.contains(&board("sleepy")), "{first}");
    let back = agent.restarted("demo@local", &before).await;
    back_after(&back, 1, GuestExitCause::Killed, "did not answer in time");
    let sleepy = agent
        .wait_guest("sleepy@local", "its kill", |status| {
            status.last_exit.is_some()
        })
        .await;
    assert_eq!(
        sleepy.last_exit.unwrap().cause,
        GuestExitCause::Killed,
        "{:#?}",
        sleepy.error
    );
    // Off, it no longer holds up the turns that follow.
    agent
        .service
        .set_package_enabled("sleepy@local", false)
        .await
        .unwrap();

    agent.echo_turn("other", "other echoed.").await;
    agent.echo_turn("demo", "demo echoed.").await;
    agent.finish().await;
}

pub async fn a_guest_that_aborts_segfaults_or_runs_out_of_memory_restarts() {
    if !sandboxed() {
        return;
    }
    // Each failure in a guest of its own, so each is its first fault.
    let crashes = [
        ("aborts", "abort", "SIGABRT"),
        ("segfaults", "segfault", "SIGSEGV"),
        ("starved", "oom", "SIGABRT"),
    ];
    let mut script: Vec<Vec<u8>> = crashes
        .iter()
        .flat_map(|(plugin, tool, _)| failing_turn(plugin, tool))
        .collect();
    script.extend(echo_turn("other"));
    script.extend(crashes.iter().flat_map(|(plugin, _, _)| echo_turn(plugin)));
    let agent = Agent::start("crashes-abort", script).await;
    let mut before = Vec::new();
    for (plugin, _, _) in crashes {
        before.push(agent.install_guest(plugin, "run").await);
    }
    agent.install_guest("other", "run").await;

    for ((plugin, tool, signal), before) in crashes.iter().zip(&before) {
        fail_in_a_turn(&agent, plugin, tool).await;
        let back = agent.restarted(&format!("{plugin}@local"), before).await;
        back_after(&back, 1, GuestExitCause::Exited, signal);
    }

    // `kill -9` from outside, as the system's out-of-memory killer does.
    #[cfg(unix)]
    {
        let killed = agent.guest("other@local").await;
        crate::fixture::kill_hard(killed.process_id.unwrap());
        let back = agent.restarted("other@local", &killed).await;
        back_after(&back, 1, GuestExitCause::Exited, "SIGKILL");
    }

    agent.echo_turn("other", "other echoed.").await;
    for (plugin, _, _) in crashes {
        agent.echo_turn(plugin, &format!("{plugin} echoed.")).await;
    }
    agent.finish().await;
}

pub async fn protocol_garbage_or_an_oversize_frame_is_a_guest_crash() {
    if !sandboxed() {
        return;
    }
    let crashes = [
        ("garbled", "garbage", "invalid message from guest"),
        ("oversized", "huge", "exceeds the limit"),
    ];
    let mut script: Vec<Vec<u8>> = crashes
        .iter()
        .flat_map(|(plugin, tool, _)| failing_turn(plugin, tool))
        .collect();
    script.extend(echo_turn("other"));
    script.extend(crashes.iter().flat_map(|(plugin, _, _)| echo_turn(plugin)));
    let agent = Agent::start("crashes-protocol", script).await;
    let mut before = Vec::new();
    for (plugin, _, _) in crashes {
        before.push(agent.install_guest(plugin, "run").await);
    }
    agent.install_guest("other", "run").await;

    for ((plugin, tool, why), before) in crashes.iter().zip(&before) {
        fail_in_a_turn(&agent, plugin, tool).await;
        let back = agent.restarted(&format!("{plugin}@local"), before).await;
        back_after(&back, 1, GuestExitCause::Killed, why);
    }

    agent.echo_turn("other", "other echoed.").await;
    for (plugin, _, _) in crashes {
        agent.echo_turn(plugin, &format!("{plugin} echoed.")).await;
    }
    agent.finish().await;
}

pub async fn a_restart_loop_leaves_the_guest_failed_until_it_is_retried() {
    if !sandboxed() {
        return;
    }
    let mut script: Vec<Vec<u8>> = (0..3)
        .flat_map(|_| failing_turn("looping", "exit"))
        .collect();
    script.extend(failing_turn("looping", "abort"));
    script.extend(echo_turn("other"));
    script.extend(echo_turn("looping"));
    let agent = Agent::start("crashes-restart-loop", script).await;
    let mut current = agent.install_guest("looping", "run").await;
    agent.install_guest("other", "run").await;

    // Three faults, each restarted after the policy's delay (2 s, 10 s,
    // 30 s).
    for restarts in 1..=3 {
        fail_in_a_turn(&agent, "looping", "exit").await;
        current = agent.restarted("looping@local", &current).await;
        back_after(&current, restarts, GuestExitCause::Exited, "exit status: 5");
    }
    // The fourth in the window: failed, and shown so, until a retry.
    fail_in_a_turn(&agent, "looping", "abort").await;
    let failed = agent
        .wait_guest("looping@local", "its failure", |status| {
            status.state == GuestState::Failed
        })
        .await;
    assert_eq!(failed.restarts, 3, "{failed:#?}");
    assert!(failed.error.is_some(), "{failed:#?}");
    assert_eq!(failed.process_id, None);

    // A turn meanwhile goes without it.
    agent.echo_turn("other", "other echoed.").await;
    let request = agent.last_request();
    assert!(!request.contains(&board("looping")), "{request}");
    assert!(request.contains(&board("other")), "{request}");
    let failed = agent.guest("looping@local").await;
    assert_eq!(failed.state, GuestState::Failed, "{failed:#?}");

    agent.service.retry_package("looping@local").await.unwrap();
    let retried = agent
        .wait_guest("looping@local", "its start after the retry", running)
        .await;
    assert_eq!(retried.restarts, 0);
    agent.echo_turn("looping", "looping echoed.").await;
    assert!(agent.last_request().contains(&board("looping")));
    agent.finish().await;
}
