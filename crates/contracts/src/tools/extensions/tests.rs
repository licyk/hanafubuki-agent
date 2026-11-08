use std::sync::Arc;

use super::TurnExtensions;

#[derive(Debug, PartialEq)]
struct Catalog(u32);

#[derive(Debug, PartialEq)]
struct Other;

#[test]
fn values_are_found_by_their_type_and_replaced_by_a_value_of_the_same_type() {
    let catalog = Arc::new(Catalog(1));
    let mut extensions = TurnExtensions::default().with(catalog.clone());
    assert_eq!(extensions.get::<Catalog>(), Some(&Catalog(1)));
    assert!(Arc::ptr_eq(
        &extensions.get_arc::<Catalog>().unwrap(),
        &catalog
    ));
    assert_eq!(extensions.get::<Other>(), None);

    extensions.insert(Arc::new(Catalog(2)));
    assert_eq!(extensions.get::<Catalog>(), Some(&Catalog(2)));
}

#[test]
fn a_disjoint_merge_adds_everything_and_an_overlapping_one_adds_nothing() {
    let mut extensions = TurnExtensions::default().with(Arc::new(Catalog(1)));
    assert!(extensions.merge_disjoint(TurnExtensions::default().with(Arc::new(Other))));
    assert_eq!(extensions.get::<Other>(), Some(&Other));

    let overlapping = TurnExtensions::default()
        .with(Arc::new(Catalog(2)))
        .with(Arc::new(7u8));
    assert!(!extensions.merge_disjoint(overlapping));
    assert_eq!(extensions.get::<Catalog>(), Some(&Catalog(1)));
    assert_eq!(extensions.get::<u8>(), None);
}
