//! A center's one writer, and what a send does.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use granite::{Center, Effect, Event, FilePersistence, GraniteReducer, Lifecycle, Main};

#[derive(Clone, PartialEq, Default, Debug, serde::Serialize, serde::Deserialize)]
struct Counts {
    total: u32,
    sends: u32,
    effects_saw: Vec<u32>,
}

struct Add;
impl GraniteReducer for Add {
    type State = Counts;
    type Payload = u32;
    fn reduce(&self, state: &mut Counts, payload: &u32) {
        state.total += payload;
        state.sends += 1;
    }
}

/// Reduces, then its effect reads the committed state and sends again through a sibling.
struct AddThenRecord(Event<Record>);
struct Record;
impl GraniteReducer for Record {
    type State = Counts;
    type Payload = u32;
    fn reduce(&self, state: &mut Counts, payload: &u32) {
        state.effects_saw.push(*payload);
    }
}
impl GraniteReducer for AddThenRecord {
    type State = Counts;
    type Payload = u32;
    fn reduce(&self, state: &mut Counts, payload: &u32) {
        state.total += payload;
    }
    fn effect(&self, state: &Counts, _payload: u32) -> Effect {
        Effect::chain(&self.0, state.total)
    }
}

#[test]
fn a_send_commits_one_main_turn_later_and_in_order() {
    let main = Main::spawn();
    let center = Center::<Counts>::new(&main);
    let add = Event::new(&center, Add);
    main.block_on(async {
        for n in 1..=5 {
            add.send(n);
        }
        assert_eq!(
            center.state().sends,
            0,
            "nothing is written before main's turn"
        );
        main.turn().await;
        assert_eq!(
            center.state(),
            Counts {
                total: 15,
                sends: 5,
                effects_saw: vec![]
            }
        );
    });
}

#[test]
fn an_effect_runs_after_the_commit_and_sends_again() {
    let main = Main::spawn();
    let center = Center::<Counts>::new(&main);
    let record = Event::new(&center, Record);
    let add = Event::new(&center, AddThenRecord(record));
    main.block_on(async {
        add.send(2);
        add.send(3);
        assert!(
            granite::testing::until(
                || center.state().effects_saw.len() == 2,
                Duration::from_secs(2)
            )
            .await
        );
        let state = center.state();
        assert_eq!(state.total, 5);
        let mut saw = state.effects_saw;
        saw.sort();
        assert_eq!(
            saw,
            vec![2, 5],
            "each effect read the state its own commit wrote"
        );
    });
}

#[test]
fn a_commit_that_changes_nothing_wakes_no_one() {
    let main = Main::spawn();
    let center = Center::<Counts>::new(&main);
    let watch = center.watch();
    main.block_on(async {
        let c = center.clone();
        main.call(move || c.commit(Counts::default())).await;
        assert!(!watch.has_changed().unwrap());
        let c = center.clone();
        main.call(move || c.update(|state| state.total = 1)).await;
        assert!(watch.has_changed().unwrap());
    });
}

#[test]
fn a_debounced_event_sends_the_last_of_a_burst() {
    let main = Main::spawn();
    let center = Center::<Counts>::new(&main);
    let add = Event::new(&center, Add).debounced(Duration::from_millis(40));
    main.block_on(async {
        for n in 1..=5 {
            add.send(n);
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        main.turn().await;
        assert_eq!(
            center.state(),
            Counts {
                total: 5,
                sends: 1,
                effects_saw: vec![]
            }
        );
    });
}

#[test]
fn a_throttled_event_keeps_the_first_and_then_the_latest() {
    let main = Main::spawn();
    let center = Center::<Counts>::new(&main);
    let add = Event::new(&center, Add).throttled(Duration::from_millis(60), true);
    main.block_on(async {
        for n in 1..=4 {
            add.send(n);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        main.turn().await;
        assert_eq!(
            center.state(),
            Counts {
                total: 5,
                sends: 2,
                effects_saw: vec![]
            }
        );
    });
}

#[test]
fn a_throttled_event_without_latest_keeps_the_first_of_the_window() {
    let main = Main::spawn();
    let center = Center::<Counts>::new(&main);
    let add = Event::new(&center, Add).throttled(Duration::from_millis(60), false);
    main.block_on(async {
        for n in 1..=4 {
            add.send(n);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        main.turn().await;
        assert_eq!(
            center.state(),
            Counts {
                total: 3,
                sends: 2,
                effects_saw: vec![]
            },
            "1 at once, then 2: the first held back, as Swift's throttle keeps it"
        );
    });
}

/// A payload that is itself an `Option`: a `None` held back is still a payload.
struct Maybe;
impl GraniteReducer for Maybe {
    type State = Counts;
    type Payload = Option<u32>;
    fn reduce(&self, state: &mut Counts, payload: &Option<u32>) {
        state.total += payload.unwrap_or(100);
        state.sends += 1;
    }
}

#[test]
fn a_none_payload_held_back_is_delivered() {
    let main = Main::spawn();
    let center = Center::<Counts>::new(&main);
    let throttled = Event::new(&center, Maybe).throttled(Duration::from_millis(40), true);
    let debounced = Event::new(&center, Maybe).debounced(Duration::from_millis(20));
    main.block_on(async {
        throttled.send(Some(1));
        throttled.send(None);
        debounced.send(Some(5));
        debounced.send(None);
        tokio::time::sleep(Duration::from_millis(150)).await;
        main.turn().await;
        assert_eq!(
            center.state(),
            Counts {
                total: 201,
                sends: 3,
                effects_saw: vec![]
            }
        );
    });
}

#[test]
fn clones_of_an_event_share_one_delivery() {
    let main = Main::spawn();
    let center = Center::<Counts>::new(&main);
    let add = Event::new(&center, Add);
    let debounced = add.clone().debounced(Duration::from_millis(40));
    main.block_on(async {
        add.send(1);
        debounced.send(2);
        add.send(3);
        tokio::time::sleep(Duration::from_millis(150)).await;
        main.turn().await;
        assert_eq!(
            center.state(),
            Counts {
                total: 3,
                sends: 1,
                effects_saw: vec![]
            },
            "one burst across both handles, and the last of it"
        );
    });
}

#[test]
fn lifecycle_hooks_fire_on_main_when_the_center_appears() {
    let main = Main::spawn();
    let center = Center::<Counts>::new(&main);
    let fired = Arc::new(AtomicUsize::new(0));
    let (seen, m) = (fired.clone(), main.clone());
    center.on_lifecycle(Lifecycle::OnAppear, move || {
        assert!(m.is_main());
        seen.fetch_add(1, Ordering::SeqCst);
    });
    let _add = Event::new(&center, Add).lifecycle(Lifecycle::OnTask);
    main.block_on(async {
        center.appear();
        center.perform_tasks();
        main.turn().await;
        main.turn().await;
        assert_eq!(fired.load(Ordering::SeqCst), 1);
        assert_eq!(
            center.state().sends,
            1,
            "the OnTask event sent its default payload"
        );
    });
}

#[test]
fn a_persisted_center_reads_what_it_wrote() {
    let main = Main::spawn();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("counts.json");
    let center = Center::<Counts>::persisted(&main, Arc::new(FilePersistence::new(&path)));
    main.block_on(async {
        let c = center.clone();
        main.call(move || c.update(|state| state.total = 42)).await;
    });
    let again = Center::<Counts>::persisted(&main, Arc::new(FilePersistence::new(&path)));
    assert_eq!(again.state().total, 42);
}
