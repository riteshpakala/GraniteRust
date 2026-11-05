//! The agent run engine: policies, the single writer, waits, taps, merges and children.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use granite::{
    Agent, AgentConcurrency, AgentContext, AgentLifecycle, AgentRunError, AgentRunId, AgentStart,
    BoxError, Center, ChildFrame, GraniteAgent, Lifecycle, Main, RunScope,
};

#[derive(Clone, PartialEq, Default, Debug)]
struct Tally {
    events: Vec<(String, u32)>,
    lifecycles: Vec<(String, String)>,
    off_main: usize,
    merged: Vec<u32>,
}

#[derive(Clone)]
struct Ask {
    key: String,
    count: u32,
    every_ms: u64,
    fail: bool,
}

fn ask(key: &str, count: u32, every_ms: u64) -> Ask {
    Ask {
        key: key.into(),
        count,
        every_ms,
        fail: false,
    }
}

fn on_main() -> bool {
    std::thread::current().name() == Some("granite.main")
}

#[derive(Default)]
struct Counter;

impl GraniteAgent for Counter {
    type State = Tally;
    type Input = Ask;
    type Event = u32;
    type Compute = Arc<Mutex<Vec<u32>>>;

    fn key(input: &Ask) -> String {
        input.key.clone()
    }

    fn lifecycle_input() -> Option<Ask> {
        Some(ask("lifecycle", 1, 0))
    }

    async fn run(
        &self,
        input: Ask,
        context: AgentContext<Self::Compute, u32>,
    ) -> Result<(), BoxError> {
        for n in 1..=input.count {
            tokio::time::sleep(Duration::from_millis(input.every_ms)).await;
            context.deliver(n);
        }
        if input.fail {
            return Err("told to fail".into());
        }
        Ok(())
    }

    fn reduce(&self, state: &mut Tally, event: u32, run: &AgentRunId) {
        if !on_main() {
            state.off_main += 1;
        }
        state.events.push((run.raw.clone(), event));
    }

    fn reduce_lifecycle(&self, state: &mut Tally, lifecycle: &AgentLifecycle, run: &AgentRunId) {
        state
            .lifecycles
            .push((run.raw.clone(), lifecycle.to_string()));
    }

    async fn merge(
        &self,
        input: Ask,
        _run: &AgentRunId,
        context: &AgentContext<Self::Compute, u32>,
    ) {
        context.compute.lock().unwrap().push(input.count);
    }
}

fn counter(
    main: &Main,
    policy: AgentConcurrency,
) -> (Center<Tally>, Agent<Counter>, Arc<Mutex<Vec<u32>>>) {
    let center = Center::<Tally>::new(main);
    let agent = Agent::<Counter>::with_concurrency(&center, policy);
    let merges = Arc::new(Mutex::new(Vec::new()));
    agent.attach_compute(merges.clone());
    (center, agent, merges)
}

fn lifecycles(center: &Center<Tally>, run: &str) -> Vec<String> {
    center
        .state()
        .lifecycles
        .into_iter()
        .filter(|(id, _)| id == run)
        .map(|(_, said)| said)
        .collect()
}

#[test]
fn a_run_streams_into_its_center_on_main_and_perform_returns_after_the_last_event() {
    let main = Main::spawn();
    let (center, agent, _) = counter(&main, AgentConcurrency::Supersede);
    main.block_on(async {
        let outcome = agent.perform(ask("a", 3, 2)).await.unwrap();
        assert_eq!(outcome.frames, 3);
        let state = center.state();
        assert_eq!(
            state.events,
            vec![
                ("Counter-1".into(), 1),
                ("Counter-1".into(), 2),
                ("Counter-1".into(), 3)
            ]
        );
        assert_eq!(
            lifecycles(&center, "Counter-1"),
            vec!["started", "finished"]
        );
        assert_eq!(state.off_main, 0, "every event was reduced on main");
    });
}

#[test]
fn supersede_cancels_the_live_run_and_drops_its_late_events() {
    let main = Main::spawn();
    let (center, agent, _) = counter(&main, AgentConcurrency::Supersede);
    main.block_on(async {
        let first = agent.clone();
        let first = tokio::spawn(async move { first.perform(ask("k", 5, 30)).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        agent.perform(ask("k", 2, 1)).await.unwrap();
        assert_eq!(
            first.await.unwrap(),
            Err(AgentRunError::Superseded(AgentRunId::new(
                "Counter-1",
                "k",
                None
            )))
        );
        tokio::time::sleep(Duration::from_millis(60)).await;
        main.turn().await;
        assert_eq!(
            lifecycles(&center, "Counter-1"),
            vec!["started", "cancelled"]
        );
        assert!(
            center
                .state()
                .events
                .iter()
                .all(|(run, _)| run == "Counter-2"),
            "no event of the superseded run was reduced"
        );
    });
}

#[test]
fn drop_ignores_a_second_send_while_the_first_runs() {
    let main = Main::spawn();
    let (_center, agent, _) = counter(&main, AgentConcurrency::Drop);
    main.block_on(async {
        assert!(matches!(
            agent.start(ask("k", 3, 20)).await,
            AgentStart::Started(_)
        ));
        assert_eq!(agent.start(ask("k", 1, 0)).await, AgentStart::Dropped);
        assert_eq!(
            agent.perform(ask("k", 1, 0)).await,
            Err(AgentRunError::Dropped("Counter".into()))
        );
    });
}

#[test]
fn parallel_runs_up_to_max_and_a_queued_run_keeps_its_id() {
    let main = Main::spawn();
    let (center, agent, _) = counter(&main, AgentConcurrency::Parallel { max: Some(2) });
    main.block_on(async {
        assert!(matches!(
            agent.start(ask("a", 2, 20)).await,
            AgentStart::Started(_)
        ));
        assert!(matches!(
            agent.start(ask("b", 2, 20)).await,
            AgentStart::Started(_)
        ));
        let queued = agent.start(ask("c", 1, 1)).await;
        let AgentStart::Queued(run) = queued else {
            panic!("the third run waits for a lane: {queued:?}")
        };
        assert_eq!(agent.live_keys().len(), 3);
        let outcome = agent.wait(&run).await.unwrap();
        assert_eq!(outcome.run, run);
        assert_eq!(lifecycles(&center, &run.raw), vec!["started", "finished"]);
    });
}

#[test]
fn cancel_commits_cancelled_at_once_and_fails_the_wait() {
    let main = Main::spawn();
    let (center, agent, _) = counter(&main, AgentConcurrency::Supersede);
    main.block_on(async {
        let performer = agent.clone();
        let performing = tokio::spawn(async move { performer.perform(ask("k", 10, 50)).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        agent.cancel("k");
        assert_eq!(performing.await.unwrap(), Err(AgentRunError::Cancelled));
        assert_eq!(
            lifecycles(&center, "Counter-1"),
            vec!["started", "cancelled"]
        );
        assert!(!agent.is_live("k"));
    });
}

#[test]
fn a_failed_run_says_why() {
    let main = Main::spawn();
    let (center, agent, _) = counter(&main, AgentConcurrency::Supersede);
    main.block_on(async {
        let failed = agent
            .perform(Ask {
                fail: true,
                ..ask("k", 1, 0)
            })
            .await;
        assert_eq!(
            failed,
            Err(AgentRunError::Failed(
                AgentRunId::new("Counter-1", "k", None),
                "told to fail".into()
            ))
        );
        assert_eq!(
            lifecycles(&center, "Counter-1"),
            vec!["started", "failed: told to fail"]
        );
    });
}

#[test]
fn a_dropped_performer_cancels_the_run_it_started() {
    let main = Main::spawn();
    let (center, agent, _) = counter(&main, AgentConcurrency::Supersede);
    main.block_on(async {
        let waited =
            tokio::time::timeout(Duration::from_millis(20), agent.perform(ask("k", 10, 40))).await;
        assert!(waited.is_err());
        assert!(granite::testing::until(|| !agent.is_live("k"), Duration::from_secs(2)).await);
        assert_eq!(
            lifecycles(&center, "Counter-1"),
            vec!["started", "cancelled"]
        );
    });
}

#[test]
fn a_tap_sees_every_event_and_ends_with_the_run() {
    let main = Main::spawn();
    let (_center, agent, _) = counter(&main, AgentConcurrency::Supersede);
    main.block_on(async {
        let tap = agent.events("k");
        agent.perform(ask("k", 4, 1)).await.unwrap();
        assert_eq!(tap.collect().await, vec![1, 2, 3, 4]);
    });
}

#[test]
fn merge_hands_a_second_send_to_the_live_run() {
    let main = Main::spawn();
    let (_center, agent, merges) = counter(&main, AgentConcurrency::Merge);
    main.block_on(async {
        let AgentStart::Started(run) = agent.start(ask("k", 3, 20)).await else {
            panic!("started")
        };
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert_eq!(
            agent.start(ask("k", 77, 0)).await,
            AgentStart::Merged(run.clone())
        );
        agent.wait(&run).await.unwrap();
        assert_eq!(*merges.lock().unwrap(), vec![77]);
    });
}

#[test]
fn a_send_without_compute_fails_with_the_sentence() {
    let main = Main::spawn();
    let center = Center::<Tally>::new(&main);
    let agent = Agent::<Counter>::new(&center);
    main.block_on(async {
        let failed = agent.perform(ask("k", 1, 0)).await;
        let Err(AgentRunError::Failed(_, said)) = failed else {
            panic!("{failed:?}")
        };
        assert!(
            said.starts_with("Counter has no compute attached"),
            "{said}"
        );
    });
}

#[test]
fn wait_on_a_run_that_ended_unwatched_is_unknown() {
    let main = Main::spawn();
    let (_center, agent, _) = counter(&main, AgentConcurrency::Supersede);
    main.block_on(async {
        let AgentStart::Started(run) = agent.start(ask("k", 1, 0)).await else {
            panic!("started")
        };
        assert!(granite::testing::until(|| !agent.is_live("k"), Duration::from_secs(2)).await);
        assert_eq!(agent.wait(&run).await, Err(AgentRunError::UnknownRun(run)));
    });
}

#[test]
fn wait_prefix_returns_when_every_matching_run_ended() {
    let main = Main::spawn();
    let (center, agent, _) = counter(&main, AgentConcurrency::Parallel { max: None });
    main.block_on(async {
        agent.send(ask("s/1", 2, 10));
        agent.send(ask("s/2", 3, 10));
        agent.send(ask("t/1", 50, 50));
        main.turn().await;
        agent.wait_prefix("s/").await;
        let ended: Vec<String> = center
            .state()
            .lifecycles
            .into_iter()
            .filter(|(_, said)| said == "finished")
            .map(|(run, _)| run)
            .collect();
        assert_eq!(ended.len(), 2);
        agent.cancel_prefix("t/");
    });
}

#[test]
fn the_lifecycle_starts_a_run_with_its_input() {
    let main = Main::spawn();
    let center = Center::<Tally>::new(&main);
    let agent = Agent::<Counter>::new(&center).lifecycle(Lifecycle::OnTask);
    agent.attach_compute(Arc::new(Mutex::new(Vec::new())));
    main.block_on(async {
        center.perform_tasks();
        assert!(
            granite::testing::until(|| center.state().events.len() == 1, Duration::from_secs(2))
                .await
        );
    });
}

#[derive(Default)]
struct Reentrant;

#[derive(Clone, PartialEq, Default, Debug)]
struct Said(Vec<String>);

impl GraniteAgent for Reentrant {
    type State = Said;
    type Input = Option<Agent<Reentrant>>;
    type Event = String;
    type Compute = ();

    async fn run(
        &self,
        input: Self::Input,
        context: AgentContext<(), String>,
    ) -> Result<(), BoxError> {
        assert!(RunScope::current().is_some());
        if let Some(myself) = input {
            match myself.perform(None).await {
                Err(AgentRunError::Reentrant(_)) => context.deliver("refused".into()),
                other => context.deliver(format!("{other:?}")),
            }
        }
        Ok(())
    }

    fn reduce(&self, state: &mut Said, event: String, _run: &AgentRunId) {
        state.0.push(event)
    }
}

#[test]
fn perform_from_inside_its_own_run_is_refused() {
    let main = Main::spawn();
    let center = Center::<Said>::new(&main);
    let agent = Agent::<Reentrant>::new(&center);
    agent.attach_compute(());
    main.block_on(async {
        agent.perform(Some(agent.clone())).await.unwrap();
        assert_eq!(center.state().0, vec!["refused".to_string()]);
    });
}

#[derive(Default)]
struct Parent;

impl GraniteAgent for Parent {
    type State = Said;
    type Input = u32;
    type Event = String;
    type Compute = ();

    async fn run(&self, input: u32, context: AgentContext<(), String>) -> Result<(), BoxError> {
        context.deliver("parent began".into());
        let mut child = context.spawn(
            Counter,
            ask("child", input, 1),
            Arc::new(Mutex::new(Vec::new())),
            "child-1",
            "counter",
            "count",
            |frame| {
                Some(match frame {
                    ChildFrame::Started { run, kind, .. } => format!("{run} started as {kind}"),
                    ChildFrame::Event(_, n) => format!("child said {n}"),
                    ChildFrame::Finished(run, ended) => format!("{run} {ended}"),
                })
            },
        );
        while let Some(said) = child.next().await {
            context.deliver(said);
        }
        Ok(())
    }

    fn reduce(&self, state: &mut Said, event: String, _run: &AgentRunId) {
        state.0.push(event)
    }
}

#[test]
fn a_child_is_announced_through_its_parent_in_the_parents_order() {
    let main = Main::spawn();
    let center = Center::<Said>::new(&main);
    let agent = Agent::<Parent>::new(&center);
    agent.attach_compute(());
    main.block_on(async {
        agent.perform(2).await.unwrap();
        assert_eq!(
            center.state().0,
            vec![
                "parent began",
                "child-1 started as counter",
                "child said 1",
                "child said 2",
                "child-1 finished"
            ]
        );
    });
}
