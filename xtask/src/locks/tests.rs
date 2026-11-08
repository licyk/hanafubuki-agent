use super::*;

fn lines(source: &str) -> Vec<usize> {
    violations(source)
        .into_iter()
        .map(|(line, _)| line)
        .collect()
}

#[test]
fn single_and_grouped_imports_are_found() {
    assert_eq!(lines("use std::sync::Mutex;\n"), vec![1]);
    assert_eq!(lines("use std::sync::RwLock;\n"), vec![1]);
    assert_eq!(lines("use std::sync::{Arc, Mutex};\n"), vec![1]);
    assert_eq!(lines("use ::std::sync::MutexGuard;\n"), vec![1]);
    assert_eq!(
        lines("use std::sync::Mutex as StdMutex;\nfn f(_: StdMutex<()>) {}\n"),
        vec![1]
    );
}

#[test]
fn multi_line_and_nested_imports_are_found() {
    let source = "\
use std::{
    collections::HashMap,
    sync::{
        atomic::AtomicBool,
        Arc,
        RwLock,
    },
};
";
    assert_eq!(lines(source), vec![1]);
    assert_eq!(lines("use std::sync::*;\n"), vec![1]);
}

#[test]
fn inline_and_aliased_paths_are_found() {
    let source = "\
struct State {
    value: std::sync::Mutex<u64>,
}
fn make() -> std::sync::Arc<std::sync::RwLock<u8>> {
    std::sync::Arc::new(std::sync::RwLock::new(0))
}
";
    assert_eq!(lines(source), vec![2, 4, 5]);

    let aliased = "\
use std::sync;
fn make() -> sync::Mutex<u8> {
    sync::Mutex::new(0)
}
";
    assert_eq!(lines(aliased), vec![2, 3]);
}

#[test]
fn macro_bodies_are_scanned() {
    assert_eq!(
        lines("fn f() {\n    let _ = vec![std::sync::Mutex::new(1)];\n}\n"),
        vec![2]
    );
}

#[test]
fn other_locks_and_prose_are_allowed() {
    let source = "\
//! Never `std::sync::Mutex`: it is poisoned by a panic.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use parking_lot::{Condvar, Mutex, RwLock};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync;

/// A `std::sync::RwLock` would poison here.
struct State {
    value: Mutex<u64>,
    shared: Arc<RwLock<u8>>,
    wake: Condvar,
    queue: sync::Mutex<u8>,
    async_value: AsyncMutex<u8>,
    flag: AtomicBool,
}
fn f() {
    let _ = format!(\"std::sync::Mutex\");
    let _ = parking_lot::Mutex::new(0);
}
";
    assert!(violations(source).is_empty(), "{:?}", violations(source));
}
