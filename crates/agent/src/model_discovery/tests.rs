use super::*;

const HOUR: Duration = Duration::from_secs(60 * 60);
const MINUTE: Duration = Duration::from_secs(60);
const NOW_MS: u64 = 1_000_000_000_000;

/// A now far enough ahead that a day before it is an `Instant` (on Windows
/// one cannot be earlier than the boot).
fn ahead() -> Instant {
    Instant::now() + 48 * HOUR
}

fn policy() -> ModelDiscoveryPolicy {
    ModelDiscoveryPolicy::every(24 * HOUR, 15 * MINUTE)
}

fn attempt(ago: Duration, succeeded: bool, now: Instant) -> Attempt {
    Attempt {
        at: now - ago,
        succeeded,
    }
}

fn ms(duration: Duration) -> u64 {
    duration.as_millis() as u64
}

#[test]
fn an_endpoint_never_listed_is_due() {
    assert!(due(&policy(), None, None, ahead(), NOW_MS));
}

#[test]
fn a_listing_holds_the_next_off_for_a_day_and_a_failure_for_its_backoff() {
    let now = ahead();
    let succeeded = attempt(23 * HOUR, true, now);
    assert!(!due(&policy(), Some(&succeeded), None, now, NOW_MS));
    let succeeded = attempt(24 * HOUR, true, now);
    assert!(due(&policy(), Some(&succeeded), None, now, NOW_MS));

    let failed = attempt(14 * MINUTE, false, now);
    assert!(!due(&policy(), Some(&failed), None, now, NOW_MS));
    let failed = attempt(15 * MINUTE, false, now);
    assert!(due(&policy(), Some(&failed), None, now, NOW_MS));
}

/// The endpoint's recorded listing (an earlier run's, or one the person
/// asked for) counts as a success, also after this run's failure.
#[test]
fn a_recorded_listing_holds_the_next_off_for_a_day() {
    let now = ahead();
    let recent = Some(NOW_MS - ms(HOUR));
    assert!(!due(&policy(), None, recent, now, NOW_MS));
    let failed = attempt(HOUR, false, now);
    assert!(!due(&policy(), Some(&failed), recent, now, NOW_MS));

    let old = Some(NOW_MS - ms(25 * HOUR));
    assert!(due(&policy(), Some(&failed), old, now, NOW_MS));
    // A clock set back does not hold discovery off.
    let future = Some(NOW_MS + ms(HOUR));
    assert!(due(&policy(), None, future, now, NOW_MS));
}
