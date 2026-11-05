//! Signals, subscriptions and scopes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use granite::{Main, Scope, Signal};

#[test]
fn a_signal_fans_out_in_order_and_drop_ends_an_observation() {
    let signal = Signal::<u32>::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (a, b) = (seen.clone(), seen.clone());
    let first = signal.observe(move |n| a.lock().unwrap().push(("first", n)));
    let second = signal.observe(move |n| b.lock().unwrap().push(("second", n)));
    signal.send(1);
    drop(second);
    signal.send(2);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![("first", 1), ("second", 1), ("first", 2)]
    );
    assert_eq!(signal.observer_count(), 1);
    assert_eq!(signal.last_value(), Some(2));
    first.forget();
    assert_eq!(signal.observer_count(), 1, "a forgotten observation stays");
}

#[test]
fn an_observer_of_changes_hears_the_value_before() {
    let signal = Signal::seeded(1u32);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let kept = seen.clone();
    let _observing =
        signal.observe_change(move |now, before| kept.lock().unwrap().push((now, before)));
    signal.send(2);
    signal.send(3);
    assert_eq!(*seen.lock().unwrap(), vec![(2, Some(1)), (3, Some(2))]);
}

#[test]
fn a_handler_may_send_on_its_own_signal() {
    let signal = Signal::<u32>::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (again, kept) = (signal.clone(), seen.clone());
    let _observing = signal.observe(move |n| {
        kept.lock().unwrap().push(n);
        if n < 3 {
            again.send(n + 1)
        }
    });
    signal.send(1);
    assert_eq!(*seen.lock().unwrap(), vec![1, 2, 3]);
}

#[test]
fn a_scope_cancel_is_terminal_and_reaches_its_children() {
    let signal = Signal::<u32>::new();
    let root = Scope::root("root");
    let child = root.child("child");
    signal.observe(|_| {}).store_in(&root);
    signal.observe(|_| {}).store_in(&child);
    assert_eq!(signal.observer_count(), 2);
    root.cancel();
    assert_eq!(signal.observer_count(), 0);
    assert!(child.is_cancelled());
    signal.observe(|_| {}).store_in(&root);
    assert_eq!(
        signal.observer_count(),
        0,
        "what a cancelled scope is given is cancelled at once"
    );
    assert!(root.dump().contains("root/child"));
}

#[test]
fn a_debounced_signal_sends_the_last_of_a_burst() {
    let main = Main::spawn();
    let signal = Signal::<u32>::new();
    let quiet = signal.debounced(Duration::from_millis(30), &main);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let kept = seen.clone();
    let _observing = quiet.observe(move |n| kept.lock().unwrap().push(n));
    main.block_on(async {
        for n in 1..=4 {
            signal.send(n);
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    });
    assert_eq!(*seen.lock().unwrap(), vec![4]);
}
