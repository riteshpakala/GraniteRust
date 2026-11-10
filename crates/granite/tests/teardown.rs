//! What ends with its last handle: an event or an agent hooked to its center's lifecycle, a payload a
//! timer holds back, and what a timer replaces or refuses. Nothing here is cancelled; it is released.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use granite::{
    Agent, AgentContext, AgentRunId, BoxError, Center, Event, GraniteAgent, GraniteReducer,
    GraniteService, Lifecycle, Main, Relay, TransientRegistry,
};

#[derive(Clone, PartialEq, Default, Debug)]
struct Sends(u32);

/// Counts its sends, and holds `held` for as long as it lives.
struct Count {
    _held: Arc<()>,
}

impl GraniteReducer for Count {
    type State = Sends;
    type Payload = ();
    fn reduce(&self, state: &mut Sends, _payload: &()) {
        state.0 += 1;
    }
}

/// Counts its sends; its payload is whatever a test wants to watch go.
struct Hold;

impl GraniteReducer for Hold {
    type State = Sends;
    type Payload = Arc<()>;
    fn reduce(&self, state: &mut Sends, _payload: &Arc<()>) {
        state.0 += 1;
    }
}

/// Runs `body` on its own thread and fails if it has not finished in two seconds.
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
        Err(RecvTimeoutError::Timeout) => panic!("{name}: a drop reached a held lock and hung"),
    }
}

#[test]
fn an_event_hooked_to_its_center_goes_with_its_last_handle() {
    let main = Main::spawn();
    let center = Center::<Sends>::new(&main);
    let held = Arc::new(());
    let event = Event::new(
        &center,
        Count {
            _held: held.clone(),
        },
    )
    .lifecycle(Lifecycle::OnTask);
    main.block_on(async {
        center.perform_tasks();
        main.turn().await;
        main.turn().await;
        assert_eq!(center.state(), Sends(1), "the hook sends while it is held");

        drop(event);
        assert_eq!(
            Arc::strong_count(&held),
            1,
            "the center's hook did not keep the event alive"
        );
        center.perform_tasks();
        main.turn().await;
        main.turn().await;
        assert_eq!(center.state(), Sends(1), "the hook went with the event");
    });
}

#[derive(Clone, PartialEq, Default, Debug)]
struct Runs(u32);

#[derive(Default)]
struct Starts;

impl GraniteAgent for Starts {
    type State = Runs;
    type Input = String;
    type Event = ();
    type Compute = Arc<()>;

    fn key(input: &String) -> String {
        input.clone()
    }

    fn lifecycle_input() -> Option<String> {
        Some("task".into())
    }

    async fn run(
        &self,
        _input: String,
        context: AgentContext<Arc<()>, ()>,
    ) -> Result<(), BoxError> {
        context.deliver(());
        Ok(())
    }

    fn reduce(&self, state: &mut Runs, _event: (), _run: &AgentRunId) {
        state.0 += 1;
    }
}

#[test]
fn an_agent_hooked_to_its_center_goes_with_its_last_handle() {
    let main = Main::spawn();
    let center = Center::<Runs>::new(&main);
    let compute = Arc::new(());
    let agent = Agent::<Starts>::new(&center).lifecycle(Lifecycle::OnTask);
    agent.attach_compute(compute.clone());
    main.block_on(async {
        center.perform_tasks();
        assert!(
            granite::testing::until(|| center.state() == Runs(1), Duration::from_secs(2)).await
        );
        drop(agent);
        assert!(
            granite::testing::until(|| Arc::strong_count(&compute) == 1, Duration::from_secs(2))
                .await,
            "the center's hook did not keep the agent's runs alive"
        );
        center.perform_tasks();
        main.turn().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        main.turn().await;
        assert_eq!(center.state(), Runs(1), "the hook went with the agent");
    });
}

#[test]
fn a_debounced_payload_still_waiting_goes_with_its_event() {
    let main = Main::spawn();
    let center = Center::<Sends>::new(&main);
    let event = Event::new(&center, Hold).debounced(Duration::from_millis(40));
    let payload = Arc::new(());
    event.send(payload.clone());
    assert_eq!(Arc::strong_count(&payload), 2, "the event holds it back");
    drop(event);
    assert_eq!(
        Arc::strong_count(&payload),
        1,
        "the timer does not keep the event, or what it holds back"
    );
    main.block_on(async {
        tokio::time::sleep(Duration::from_millis(120)).await;
        main.turn().await;
        assert_eq!(center.state(), Sends(0));
    });
}

#[test]
fn a_throttled_payload_held_back_goes_with_its_event() {
    let main = Main::spawn();
    let center = Center::<Sends>::new(&main);
    let event = Event::new(&center, Hold).throttled(Duration::from_millis(60), true);
    let payload = Arc::new(());
    main.block_on(async {
        event.send(Arc::new(()));
        event.send(payload.clone());
        assert_eq!(Arc::strong_count(&payload), 2, "the window holds it back");
        drop(event);
        // The first send's turn holds the event until main runs it; the timer does not.
        main.turn().await;
        assert_eq!(Arc::strong_count(&payload), 1);
        tokio::time::sleep(Duration::from_millis(150)).await;
        main.turn().await;
        assert_eq!(center.state(), Sends(1), "only the first went through");
    });
}

/// Sends again on its event as it is dropped: what a timer replaces or refuses must go after the
/// event's lock is released.
struct Resend(Option<Event<Echo>>);

impl Drop for Resend {
    fn drop(&mut self) {
        if let Some(event) = self.0.take() {
            event.send(Resend(None));
        }
    }
}

struct Echo;

impl GraniteReducer for Echo {
    type State = Sends;
    type Payload = Resend;
    fn reduce(&self, state: &mut Sends, _payload: &Resend) {
        state.0 += 1;
    }
}

#[test]
fn a_debounced_payload_replaced_may_send_as_it_goes() {
    without_hanging("debounce replace", || {
        let main = Main::spawn();
        let center = Center::<Sends>::new(&main);
        let event = Event::new(&center, Echo).debounced(Duration::from_millis(20));
        event.send(Resend(Some(event.clone())));
        event.send(Resend(None));
        main.block_on(async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            main.turn().await;
        });
        assert_eq!(center.state(), Sends(1), "one delivery for the burst");
    });
}

#[test]
fn a_throttled_payload_replaced_or_refused_may_send_as_it_goes() {
    for latest in [true, false] {
        without_hanging("throttle replace or refuse", move || {
            let main = Main::spawn();
            let center = Center::<Sends>::new(&main);
            let event = Event::new(&center, Echo).throttled(Duration::from_millis(40), latest);
            // The first goes at once and the second is held back; the third is refused (`latest: false`)
            // or replaces it, and is itself replaced by the fourth. What goes at the window's end never
            // resends, so the count stays exact.
            event.send(Resend(None));
            event.send(Resend(None));
            event.send(Resend(Some(event.clone())));
            event.send(Resend(None));
            main.block_on(async {
                tokio::time::sleep(Duration::from_millis(150)).await;
                main.turn().await;
            });
            assert_eq!(
                center.state(),
                Sends(2),
                "the first, then one at the window's end"
            );
        });
    }
}

#[test]
fn a_hook_on_the_center_itself_lasts_as_long_as_the_center() {
    let main = Main::spawn();
    let center = Center::<Sends>::new(&main);
    let fired = Arc::new(Mutex::new(0));
    let seen = fired.clone();
    center.on_lifecycle(Lifecycle::OnAppear, move || *seen.lock().unwrap() += 1);
    main.block_on(async {
        center.appear();
        center.appear();
        main.turn().await;
    });
    assert_eq!(*fired.lock().unwrap(), 2);
    drop(center);
    assert_eq!(
        Arc::strong_count(&fired),
        1,
        "the hook went with the center"
    );
}

/// A resident that puts another into its registry as it is dropped.
struct Leaves(Option<Arc<TransientRegistry<Arc<Leaves>>>>);

impl Drop for Leaves {
    fn drop(&mut self) {
        if let Some(registry) = self.0.take() {
            registry.put("left behind", Arc::new(Leaves(None)));
        }
    }
}

#[test]
fn a_registry_resident_may_reach_its_registry_as_it_is_made_or_dropped() {
    without_hanging("registry", || {
        let registry = Arc::new(TransientRegistry::<Arc<Leaves>>::new());
        let made = registry.value("made", || {
            assert!(
                registry.get("other").is_none(),
                "make runs outside the lock"
            );
            Arc::new(Leaves(None))
        });
        assert!(Arc::ptr_eq(
            &made,
            &registry.value("made", || unreachable!())
        ));

        registry.put("leaving", Arc::new(Leaves(Some(registry.clone()))));
        registry.remove_all();
        assert!(
            registry.contains("left behind"),
            "the drop ran after the unlock"
        );
    });
}

/// A service made by hand, observed through a relay.
struct Tally {
    center: Center<Sends>,
}

impl GraniteService for Tally {
    type State = Sends;
    fn make(main: &Main) -> Self {
        Tally {
            center: Center::new(main),
        }
    }
    fn center(&self) -> &Center<Sends> {
        &self.center
    }
}

fn commit(relay: &Relay<Tally>, sends: u32) -> impl std::future::Future<Output = ()> + use<> {
    let center = relay.center().clone();
    let main = center.main().clone();
    async move { main.call(move || center.commit(Sends(sends))).await }
}

#[test]
fn an_observation_hears_each_change_of_what_it_selects_on_main() {
    let main = Main::spawn();
    let relay = Relay::of(Arc::new(Tally::make(&main)));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (kept, on) = (seen.clone(), main.clone());
    let _observation = relay.observe(
        |state| state.0 / 10,
        move |tens| {
            assert!(on.is_main());
            kept.lock().unwrap().push(tens)
        },
    );
    main.block_on(async {
        for sends in [1, 12, 15, 23] {
            commit(&relay, sends).await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            granite::testing::until(
                || *seen.lock().unwrap() == vec![1, 2],
                Duration::from_secs(2)
            )
            .await,
            "{:?}",
            seen.lock().unwrap()
        );
    });
}

#[test]
fn an_observation_dropped_lets_its_handler_go_and_runs_nothing_queued() {
    let main = Main::spawn();
    let relay = Relay::of(Arc::new(Tally::make(&main)));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let held = Arc::new(());
    let (kept, capture) = (seen.clone(), held.clone());
    let observation = relay.observe(
        |state| state.0,
        move |sends| {
            let _ = &capture;
            kept.lock().unwrap().push(sends)
        },
    );
    main.block_on(async {
        commit(&relay, 1).await;
        assert!(
            granite::testing::until(|| *seen.lock().unwrap() == vec![1], Duration::from_secs(2))
                .await
        );

        // Main commits, then is held, so the delivery for 2 queues behind the hold.
        let (release, hold) = mpsc::channel::<()>();
        let center = relay.center().clone();
        main.enqueue(move || {
            center.commit(Sends(2));
            let _ = hold.recv();
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(observation);
        assert_eq!(
            Arc::strong_count(&held),
            1,
            "the handler went with the observation"
        );
        release.send(()).unwrap();
        main.turn().await;
        main.turn().await;
        assert_eq!(
            *seen.lock().unwrap(),
            vec![1],
            "the queued delivery did not run"
        );
    });
}
