//! Tickets, the lane, and the registries.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use granite::{Center, Lane, Main, RunRegistry, TicketError, Tickets, TransientRegistry};

#[test]
fn a_resolve_before_the_wait_is_kept() {
    let main = Main::spawn();
    let tickets = Tickets::<u32>::new(&main);
    main.block_on(async {
        let ticket = tickets.open();
        tickets.resolve(ticket.id, 7).await;
        assert_eq!(tickets.wait(ticket.id).await, Ok(7));
        assert_eq!(tickets.pending(), 0);
    });
}

#[test]
fn a_settle_waits_for_what_main_was_sent_before_it() {
    let main = Main::spawn();
    let tickets = Tickets::<u32>::new(&main);
    let center = Center::<u32>::new(&main);
    main.block_on(async {
        let ticket = tickets.open();
        let (t, c, m) = (tickets.clone(), center.clone(), main.clone());
        tokio::spawn(async move {
            let writer = c.clone();
            m.enqueue(move || writer.commit(9));
            t.resolve(ticket.id, 1).await;
        });
        assert_eq!(tickets.wait(ticket.id).await, Ok(1));
        assert_eq!(
            center.state(),
            9,
            "the write sent before the resolve is in the state the waiter reads"
        );
    });
}

#[test]
fn a_dropped_wait_cancels_the_ticket_and_a_late_resolve_is_dropped() {
    let main = Main::spawn();
    let tickets = Tickets::<u32>::new(&main);
    main.block_on(async {
        let ticket = tickets.open();
        let waiting =
            tokio::time::timeout(Duration::from_millis(20), tickets.wait(ticket.id)).await;
        assert!(waiting.is_err());
        tickets.resolve(ticket.id, 3).await;
        assert_eq!(tickets.wait(ticket.id).await, Err(TicketError::Cancelled));
        assert_eq!(tickets.pending(), 0);
    });
}

#[test]
fn a_second_wait_is_refused_and_a_failure_reaches_the_waiter() {
    let main = Main::spawn();
    let tickets = Tickets::<u32>::new(&main);
    main.block_on(async {
        let ticket = tickets.open();
        let (t, id) = (tickets.clone(), ticket.id);
        let first = tokio::spawn(async move { t.wait(id).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(tickets.wait(id).await, Err(TicketError::AlreadyAwaited(id)));
        tickets.fail(id, TicketError::Refused("busy".into())).await;
        assert_eq!(
            first.await.unwrap(),
            Err(TicketError::Refused("busy".into()))
        );
    });
}

#[test]
fn a_lane_runs_bodies_in_arrival_order() {
    let main = Main::spawn();
    let lane = Arc::new(Lane::new());
    let order = Arc::new(Mutex::new(Vec::new()));
    main.block_on(async {
        let mut tasks = Vec::new();
        for n in 0..5u64 {
            let (lane, order) = (lane.clone(), order.clone());
            tasks.push(tokio::spawn(async move {
                lane.run(async {
                    tokio::time::sleep(Duration::from_millis(10 * (5 - n))).await;
                    order.lock().unwrap().push(n);
                })
                .await
            }));
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        for task in tasks {
            task.await.unwrap();
        }
    });
    assert_eq!(*order.lock().unwrap(), vec![0, 1, 2, 3, 4]);
}

#[test]
fn a_cancel_before_the_register_leaves_a_tombstone() {
    let main = Main::spawn();
    let registry = RunRegistry::new();
    main.block_on(async {
        registry.cancel("job");
        let task = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(5)).await });
        assert!(!registry.register("job", task));
        assert!(registry.is_cancelled("job"));
        registry.finish("job");
        assert!(!registry.is_cancelled("job"));
        let task = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(5)).await });
        assert!(registry.register("job", task));
        assert!(registry.is_running("job"));
        registry.cancel("job");
    });
}

#[test]
fn a_transient_registry_makes_one_value_per_key() {
    let registry = TransientRegistry::<u32>::new();
    assert_eq!(registry.value("a/1", || 1), 1);
    assert_eq!(registry.value("a/1", || 2), 1);
    registry.put("a/2", 2);
    registry.put("b/1", 3);
    assert_eq!(registry.keys("a/"), vec!["a/1", "a/2"]);
    let mut gone = registry.remove_all_prefixed("a/");
    gone.sort();
    assert_eq!(gone, vec![1, 2]);
    assert_eq!(registry.keys(""), vec!["b/1"]);
}
