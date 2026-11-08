use super::*;

fn fault(component: &str) -> ModuleFault {
    ModuleFault {
        owner: FaultOwner::Module("probe"),
        component: component.to_string(),
        message: "panicked".to_string(),
        at_unix_millis: 0,
    }
}

#[test]
fn the_log_counts_every_fault_and_keeps_the_last_ones_oldest_first() {
    let log = FaultLog::new(2);
    for component in ["a", "b", "c"] {
        log.record(fault(component));
    }
    let summary = log.summary();
    assert_eq!(summary.count, 3);
    assert_eq!(
        summary
            .recent
            .iter()
            .map(|fault| fault.component.as_str())
            .collect::<Vec<_>>(),
        ["b", "c"]
    );
}

#[test]
fn a_group_handler_reports_its_owner_and_the_member() {
    module_fault_handler(FaultOwner::Module("handler-probe"))("handler-probe-worker", "boom");
    let summary = recorded_module_faults();
    assert!(summary.recent.iter().any(|fault| {
        fault.owner == FaultOwner::Module("handler-probe")
            && fault.component == "handler-probe-worker"
            && fault.message == "boom"
            && fault.at_unix_millis > 0
    }));
}

#[test]
fn owners_display_as_core_or_their_module_name() {
    assert_eq!(FaultOwner::Core.to_string(), "core");
    assert_eq!(FaultOwner::Module("api-server").to_string(), "api-server");
}

#[test]
fn a_subscriber_hears_every_later_fault_until_it_is_dropped() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let sink = heard.clone();
    let subscription = subscribe_module_faults(move |fault| {
        if fault.owner == FaultOwner::Module("subscriber-probe") {
            sink.lock().push(fault.component.clone());
        }
    });
    report_module_fault(FaultOwner::Module("subscriber-probe"), "first", "boom");
    drop(subscription);
    report_module_fault(FaultOwner::Module("subscriber-probe"), "second", "boom");
    assert_eq!(*heard.lock(), ["first"]);
    // The sink records both, whoever listens.
    let recorded = recorded_module_faults();
    for component in ["first", "second"] {
        assert!(recorded.recent.iter().any(|fault| {
            fault.owner == FaultOwner::Module("subscriber-probe") && fault.component == component
        }));
    }
}
