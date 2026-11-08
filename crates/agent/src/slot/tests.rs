use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::AGENT_RUNTIME_THREAD_NAME;

#[derive(Default)]
struct CountingSink(AtomicUsize);

impl AgentStatusEventSink for CountingSink {
    fn status_changed(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_starting_slot_reports_why_the_agent_is_unavailable() {
    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    assert_eq!(slot.status().availability, AgentAvailability::Starting);
    assert_eq!(
        slot.agent().err().unwrap().to_string(),
        "the Agent is unavailable: it is still starting"
    );
    assert!(slot.begin_start().is_some());
    assert!(slot.begin_start().is_none(), "a slot starts once");
}

#[test]
fn a_disabled_slot_never_starts() {
    for reason in [AgentDisabledReason::Setting, AgentDisabledReason::CrashLoop] {
        let slot = AgentSlot::new(Some(reason), Arc::new(NoopAgentStatusEventSink));
        assert!(slot.begin_start().is_none());
        assert_eq!(
            slot.status().availability,
            AgentAvailability::Disabled { reason }
        );
        assert!(slot.agent().is_err());
    }
}

#[test]
fn the_status_is_one_flat_record_for_the_interface() {
    let events = Arc::new(CountingSink::default());
    let slot = AgentSlot::new(None, events.clone());
    assert!(slot.begin_start().is_some());
    slot.set_quarantined(vec![AgentQuarantinedSession {
        session_id: hanafubuki_agent_kernel::AgentSessionId::new(7),
        reason: "corrupt".to_string(),
    }]);
    slot.fail(AgentStartStage::Recover, "index unreadable".to_string());

    assert_eq!(
        serde_json::to_value(slot.status()).unwrap(),
        json!({
            "state": "failed",
            "stage": "recover",
            "reason": "index unreadable",
            "generation": 1,
            "safeMode": false,
            "degraded": false,
            "quarantinedSessions": [{"sessionId": 7, "reason": "corrupt"}],
        })
    );
    assert_eq!(
        slot.agent().err().unwrap().to_string(),
        "the Agent is unavailable: its session recovery failed: index unreadable"
    );
    assert_eq!(
        serde_json::to_value(
            AgentSlot::new(
                Some(AgentDisabledReason::CrashLoop),
                Arc::new(NoopAgentStatusEventSink)
            )
            .status()
        )
        .unwrap(),
        json!({
            "state": "disabled",
            "reason": "crashLoop",
            "generation": 0,
            "safeMode": false,
            "degraded": false,
            "quarantinedSessions": [],
        })
    );
    assert_eq!(events.0.load(Ordering::SeqCst), 2);
}

#[test]
fn closing_waits_for_a_start_under_way_and_no_longer() {
    // Nothing started: closing does not wait.
    let idle = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    let started = Instant::now();
    assert!(matches!(
        idle.close(Instant::now() + Duration::from_secs(5)),
        AgentClosing::NotRunning(AgentAvailability::Starting)
    ));
    assert!(started.elapsed() < Duration::from_secs(1));

    // A start that settles while shutdown waits.
    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    assert!(slot.begin_start().is_some());
    let failing = slot.clone();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        failing.fail(AgentStartStage::Construct, "late".to_string());
    });
    assert!(matches!(
        slot.close(Instant::now() + Duration::from_secs(5)),
        AgentClosing::NotRunning(AgentAvailability::Failed { .. })
    ));
    worker.join().unwrap();

    // A start that does not settle in time is abandoned; it cannot install
    // an Agent afterwards.
    let slow = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    assert!(slow.begin_start().is_some());
    assert!(matches!(
        slow.close(Instant::now() + Duration::from_millis(20)),
        AgentClosing::Abandoned
    ));
    assert!(slow.is_stopped());
    slow.fail(AgentStartStage::Start, "after close".to_string());
    assert_eq!(slow.status().availability, AgentAvailability::Stopped);
    assert_eq!(
        slow.agent().err().unwrap().to_string(),
        "the Agent is unavailable: the application is shutting down"
    );
}

#[test]
fn diagnostics_reads_nothing_from_an_absent_agent() {
    let slot = AgentSlot::new(
        Some(AgentDisabledReason::CrashLoop),
        Arc::new(NoopAgentStatusEventSink),
    );
    assert_eq!(slot.flush_for_diagnostics(), Ok(()));
    assert!(slot.plugin_status_for_diagnostics().is_empty());
    assert_eq!(
        slot.status().availability,
        AgentAvailability::Disabled {
            reason: AgentDisabledReason::CrashLoop
        }
    );
}

#[test]
fn closing_cancels_the_start_and_keeps_its_runtime_for_shutdown() {
    let slot = AgentSlot::new(None, Arc::new(NoopAgentStatusEventSink));
    let cancellation = slot.begin_start().unwrap().cancellation;
    let runtime = ContainedRuntime::start(AGENT_RUNTIME_THREAD_NAME).unwrap();
    assert!(slot
        .attach_runtime(runtime, ProcessOwnerGroup::new())
        .is_ok());
    assert!(!cancellation.is_cancelled());

    assert!(matches!(
        slot.close(Instant::now() + Duration::from_millis(20)),
        AgentClosing::Abandoned
    ));

    assert!(cancellation.is_cancelled());
    // A runtime built after the slot closed is refused and handed back.
    let late = ContainedRuntime::start(AGENT_RUNTIME_THREAD_NAME).unwrap();
    let refused = slot
        .attach_runtime(late, ProcessOwnerGroup::new())
        .unwrap_err();
    refused.shutdown(Duration::from_secs(2)).unwrap();
    // Shutdown stops the instance's runtime last, once.
    slot.take_runtime()
        .unwrap()
        .shutdown(Duration::from_secs(2))
        .unwrap();
    assert!(slot.take_runtime().is_none());
}
