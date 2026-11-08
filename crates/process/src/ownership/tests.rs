use super::*;

#[test]
fn spawning_reservation_cannot_be_finalized_before_receipt_commit() {
    let owner = ProcessOwnerId::generate();
    let mut registry = ProcessOwnershipRegistry::default();
    let reservation = registry.reserve("process-a", &owner).unwrap();

    assert!(registry.begin_spawn(&reservation));
    assert_eq!(
        registry.finalize_reservation("process-a", &owner),
        OwnedReleaseStatus::DifferentOwner
    );
    let receipt = registry.commit(&reservation).unwrap();
    assert!(registry.authorize(&receipt));
}

#[test]
fn failed_spawn_returns_reservation_to_finalizable_state() {
    let owner = ProcessOwnerId::generate();
    let mut registry = ProcessOwnershipRegistry::default();
    let reservation = registry.reserve("process-b", &owner).unwrap();

    assert!(registry.begin_spawn(&reservation));
    registry.abort_spawn(&reservation);
    assert_eq!(
        registry.finalize_reservation("process-b", &owner),
        OwnedReleaseStatus::Released
    );
}

#[test]
fn foreign_owner_cannot_finalize_or_replace_an_existing_reservation() {
    let owner = ProcessOwnerId::generate();
    let foreign_owner = ProcessOwnerId::generate();
    let mut registry = ProcessOwnershipRegistry::default();
    let reservation = registry.reserve("cross-owner", &owner).unwrap();

    assert!(registry.reserve("cross-owner", &foreign_owner).is_err());
    assert_eq!(
        registry.finalize_reservation("cross-owner", &foreign_owner),
        OwnedReleaseStatus::DifferentOwner
    );
    assert!(registry.begin_spawn(&reservation));
    let receipt = registry.commit(&reservation).unwrap();
    assert!(registry.authorize(&receipt));
}
