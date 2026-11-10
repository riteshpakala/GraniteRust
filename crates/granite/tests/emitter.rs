//! The emitter's contract, ported from Granite's `EmitterTests`: receivers are held weakly and end
//! with their owners, keys replace, delivery is ordered and re-entrant, nothing is dropped under the
//! lock, and concurrent use stays consistent.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use granite::{Emitter, EmitterKey};

/// A receiver: only its identity matters.
struct Probe;

fn probe() -> Arc<Probe> {
    Arc::new(Probe)
}

/// What the handlers saw, in order.
#[derive(Clone, Default)]
struct Recorder<T>(Arc<Mutex<Vec<T>>>);

impl<T: Clone> Recorder<T> {
    fn push(&self, value: T) {
        self.0.lock().unwrap().push(value);
    }

    fn values(&self) -> Vec<T> {
        self.0.lock().unwrap().clone()
    }
}

/// Runs `body` when it is dropped: a handler's capture that acts as it is released.
struct OnRelease(Option<Box<dyn FnOnce() + Send + Sync>>);

impl OnRelease {
    fn new(body: impl FnOnce() + Send + Sync + 'static) -> Self {
        OnRelease(Some(Box::new(body)))
    }

    fn sends(value: i32, on: &Emitter<i32>) -> Self {
        let emitter = on.clone();
        OnRelease::new(move || emitter.send(value))
    }
}

impl Drop for OnRelease {
    fn drop(&mut self) {
        if let Some(body) = self.0.take() {
            body();
        }
    }
}

/// Runs `body` on its own thread and fails if it has not finished in two seconds: std's `Mutex` is
/// not reentrant, so a release under the emitter's lock that reaches it again hangs instead of failing.
fn without_hanging(name: &str, body: impl FnOnce() + Send + 'static) {
    let (done, finished) = mpsc::channel();
    let worker = thread::spawn(move || {
        body();
        let _ = done.send(());
    });
    match finished.recv_timeout(Duration::from_secs(2)) {
        Ok(()) => worker.join().unwrap(),
        Err(RecvTimeoutError::Disconnected) => {
            std::panic::resume_unwind(worker.join().unwrap_err())
        }
        Err(RecvTimeoutError::Timeout) => panic!("{name}: a release reached a held lock and hung"),
    }
}

#[test]
fn receivers_run_in_subscription_order() {
    let emitter = Emitter::<i32>::new();
    let log = Recorder::default();
    let (first, second) = (probe(), probe());
    let (a, b) = (log.clone(), log.clone());
    emitter.on(&first, None, move |_, value| {
        a.push(format!("first {value}"))
    });
    emitter.on(&second, None, move |_, value| {
        b.push(format!("second {value}"))
    });
    emitter.send(1);
    assert_eq!(log.values(), vec!["first 1", "second 1"]);
}

#[test]
fn the_handler_is_handed_its_receiver() {
    let emitter = Emitter::<i32>::new();
    let receiver = probe();
    let expected = Arc::as_ptr(&receiver) as usize;
    let matched = Recorder::default();
    let kept = matched.clone();
    emitter.on(&receiver, None, move |handed, _| {
        kept.push(Arc::as_ptr(handed) as usize == expected)
    });
    emitter.send(1);
    assert_eq!(matched.values(), vec![true]);
}

#[test]
fn releasing_the_receiver_ends_the_subscription() {
    let emitter = Emitter::<i32>::new();
    let log = Recorder::default();
    let weak = {
        let receiver = probe();
        let kept = log.clone();
        emitter.on(&receiver, None, move |_, value| kept.push(*value));
        emitter.send(1);
        assert_eq!(emitter.receiver_count(), 1);
        Arc::downgrade(&receiver)
    };
    assert!(
        weak.upgrade().is_none(),
        "the emitter does not keep its receiver alive"
    );
    assert_eq!(emitter.receiver_count(), 0);
    emitter.send(2);
    assert_eq!(log.values(), vec![1]);
}

#[test]
fn a_key_replaces_its_subscription_and_moves_it_last() {
    let emitter = Emitter::<i32>::new();
    let log = Recorder::default();
    let (first, middle, last) = (probe(), probe(), probe());
    let key = EmitterKey::new("slot");
    let (a, b, c) = (log.clone(), log.clone(), log.clone());
    emitter.on(&first, Some(key.clone()), move |_, _| a.push("first"));
    emitter.on(&middle, None, move |_, _| b.push("middle"));
    emitter.on(&last, Some(key), move |_, _| c.push("last"));
    emitter.send(1);
    assert_eq!(log.values(), vec!["middle", "last"]);
    assert_eq!(emitter.receiver_count(), 2);
}

#[test]
fn replacing_a_key_releases_the_handler_it_replaced() {
    let emitter = Emitter::<i32>::new();
    let (first, second) = (probe(), probe());
    let key = EmitterKey::new("slot");
    let capture = Arc::new(());
    let held = capture.clone();
    emitter.on(&first, Some(key.clone()), move |_, _| {
        let _ = &held;
    });
    assert_eq!(Arc::strong_count(&capture), 2);
    emitter.on(&second, Some(key), |_, _| {});
    assert_eq!(
        Arc::strong_count(&capture),
        1,
        "the replaced handler and its captures are gone"
    );
}

#[test]
fn removing_all_ends_every_subscription() {
    let emitter = Emitter::<i32>::new();
    let log = Recorder::default();
    let (first, second) = (probe(), probe());
    let (a, b) = (log.clone(), log.clone());
    emitter.on(&first, None, move |_, value| a.push(*value));
    emitter.on(&second, None, move |_, value| b.push(*value));
    emitter.remove_all();
    emitter.send(1);
    assert_eq!(emitter.receiver_count(), 0);
    assert!(log.values().is_empty());
}

#[test]
fn off_matches_the_receiver_and_the_key() {
    let emitter = Emitter::<i32>::new();
    let log = Recorder::default();
    let (receiver, other) = (probe(), probe());
    let (a, b, c) = (log.clone(), log.clone(), log.clone());
    emitter.on(&receiver, Some(EmitterKey::new("a")), move |_, _| {
        a.push("a")
    });
    emitter.on(&receiver, Some(EmitterKey::new("b")), move |_, _| {
        b.push("b")
    });
    emitter.on(&other, Some(EmitterKey::new("c")), move |_, _| c.push("c"));

    emitter.off(&other, Some(&EmitterKey::new("a")));
    emitter.off(&receiver, Some(&EmitterKey::new("a")));
    emitter.send(1);
    assert_eq!(
        log.values(),
        vec!["b", "c"],
        "only the receiver's own subscription under the key went"
    );

    emitter.off(&receiver, None);
    emitter.send(2);
    assert_eq!(log.values(), vec!["b", "c", "c"]);
}

#[test]
fn a_receiver_released_earlier_in_the_same_send_is_not_reached() {
    let emitter = Emitter::<i32>::new();
    let log = Recorder::default();
    let first = probe();
    let holder = Arc::new(Mutex::new(Some(probe())));
    let (a, b) = (log.clone(), log.clone());
    let released = holder.clone();
    emitter.on(&first, None, move |_, _| {
        a.push("first");
        let gone = released.lock().unwrap().take();
        drop(gone);
    });
    let second = holder.lock().unwrap().clone().unwrap();
    emitter.on(&second, None, move |_, _| b.push("second"));
    drop(second);
    emitter.send(1);
    assert_eq!(log.values(), vec!["first"]);
}

#[test]
fn off_during_a_send_takes_effect_from_the_next_send() {
    let emitter = Emitter::<i32>::new();
    let log = Recorder::default();
    let (first, second) = (probe(), probe());
    let (a, b) = (log.clone(), log.clone());
    let (again, target) = (emitter.clone(), second.clone());
    emitter.on(&first, None, move |_, value| {
        a.push(format!("first {value}"));
        again.off(&target, None);
    });
    emitter.on(&second, None, move |_, value| {
        b.push(format!("second {value}"))
    });
    emitter.send(1);
    emitter.send(2);
    assert_eq!(log.values(), vec!["first 1", "second 1", "first 2"]);
}

#[test]
fn handlers_may_send_and_subscribe_reentrantly() {
    let emitter = Emitter::<i32>::new();
    let log = Recorder::default();
    let (receiver, late) = (probe(), probe());
    let (again, kept) = (emitter.clone(), log.clone());
    emitter.on(&receiver, None, move |_, value| {
        kept.push(*value);
        if *value == 1 {
            let inner = kept.clone();
            again.on(&late, None, move |_, value| inner.push(value * 10));
            again.send(2);
        }
    });
    emitter.send(1);
    assert_eq!(
        log.values(),
        vec![1, 2, 20],
        "the nested send reached the receiver added before it"
    );
    emitter.remove_all();
}

#[test]
fn a_value_less_emitter_sends() {
    let emitter = Emitter::<()>::new();
    let log = Recorder::default();
    let receiver = probe();
    let kept = log.clone();
    emitter.on(&receiver, None, move |_, ()| kept.push(true));
    emitter.send(());
    assert_eq!(log.values().len(), 1);
}

// Nothing is dropped under the lock. A handler is dropped when its subscription goes; if what it
// captured sends on the same emitter as it goes, the emitter's lock must already be free.

#[test]
fn a_handler_released_by_a_replaced_key_may_send_to_its_emitter() {
    without_hanging("replaced key", || {
        let emitter = Emitter::<i32>::new();
        let log = Recorder::default();
        let (first, second) = (probe(), probe());
        let key = EmitterKey::new("slot");
        let releases = OnRelease::sends(99, &emitter);
        emitter.on(&first, Some(key.clone()), move |_, _| {
            let _ = &releases;
        });
        let kept = log.clone();
        emitter.on(&second, Some(key), move |_, value| kept.push(*value));
        assert_eq!(
            log.values(),
            vec![99],
            "the send made during the release reached the new subscription"
        );
    });
}

#[test]
fn a_handler_released_by_off_may_send_to_its_emitter() {
    without_hanging("off", || {
        let emitter = Emitter::<i32>::new();
        let log = Recorder::default();
        let (leaving, staying) = (probe(), probe());
        let kept = log.clone();
        emitter.on(&staying, None, move |_, value| kept.push(*value));
        let releases = OnRelease::sends(7, &emitter);
        emitter.on(&leaving, None, move |_, _| {
            let _ = &releases;
        });
        emitter.off(&leaving, None);
        assert_eq!(log.values(), vec![7]);
    });
}

#[test]
fn a_handler_released_by_remove_all_may_send_to_its_emitter() {
    without_hanging("remove_all", || {
        let emitter = Emitter::<i32>::new();
        let receiver = probe();
        let releases = OnRelease::sends(1, &emitter);
        emitter.on(&receiver, None, move |_, _| {
            let _ = &releases;
        });
        emitter.remove_all();
        assert_eq!(
            emitter.receiver_count(),
            0,
            "the send found no one, and did not hang"
        );
    });
}

#[test]
fn a_handler_released_when_its_receiver_is_compacted_may_send_to_its_emitter() {
    without_hanging("compaction", || {
        let emitter = Emitter::<i32>::new();
        let log = Recorder::default();
        let staying = probe();
        let kept = log.clone();
        emitter.on(&staying, None, move |_, value| kept.push(*value));
        {
            let gone = probe();
            let releases = OnRelease::sends(5, &emitter);
            emitter.on(&gone, None, move |_, _| {
                let _ = &releases;
            });
        }
        // `send` finds the released receiver, lets its snapshot go, then compacts: the entry's
        // last handle goes in the compaction, after the unlock.
        emitter.send(0);
        assert_eq!(log.values(), vec![0, 5]);
    });
}

/// The old `Signal` deadlocked here: a derived signal's handler went under its observer's lock, and
/// the cancel it held took the same lock. A release that unsubscribes or subscribes on the emitter
/// it is leaving must find the lock free.
#[test]
fn a_release_that_changes_its_emitter_does_not_hang() {
    without_hanging("release changes its emitter", || {
        let emitter = Emitter::<i32>::new();
        let log = Recorder::default();
        let (leaving, staying, late) = (probe(), probe(), probe());
        let kept = log.clone();
        emitter.on(&staying, None, move |_, value| kept.push(*value));

        let (again, also, late_receiver) = (emitter.clone(), emitter.clone(), late.clone());
        let late_log = log.clone();
        let releases = OnRelease::new(move || {
            again.on(&late_receiver, None, move |_, value| {
                late_log.push(value * 100)
            });
            also.off(&late_receiver, Some(&EmitterKey::new("none")));
        });
        emitter.on(&leaving, None, move |_, _| {
            let _ = &releases;
        });
        emitter.off(&leaving, None);
        emitter.send(1);
        assert_eq!(log.values(), vec![1, 100]);

        let clearing = emitter.clone();
        let releases = OnRelease::new(move || clearing.remove_all());
        emitter.on(&leaving, None, move |_, _| {
            let _ = &releases;
        });
        emitter.off(&leaving, None);
        assert_eq!(emitter.receiver_count(), 0);
    });
}

#[test]
fn concurrent_send_subscribe_and_release_stay_consistent() {
    let emitter = Emitter::<i32>::new();
    let steady = probe();
    let delivered = Recorder::default();
    let kept = delivered.clone();
    emitter.on(&steady, None, move |_, value| kept.push(*value));

    let workers: Vec<_> = (0..8)
        .map(|worker| {
            let emitter = emitter.clone();
            thread::spawn(move || {
                for step in 0..125 {
                    let index = worker * 125 + step;
                    let transient = probe();
                    let key = EmitterKey::new(format!("transient {}", index % 7));
                    emitter.on(&transient, Some(key), |_, _| {});
                    emitter.send(index);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }

    emitter.send(-1);
    assert_eq!(
        delivered.values().len(),
        1_001,
        "the steady receiver heard every send"
    );
    assert_eq!(
        emitter.receiver_count(),
        1,
        "every transient receiver went with its owner"
    );
}
