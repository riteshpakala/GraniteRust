# The kernel

`crates/granite` is Swift Granite's architecture core and the agent run engine of GraniteAgents, without SwiftUI. The
rule is Granite's: each subsystem has one writer. A service's center holds its state; reducers fold sends into it on
main; work runs in effects, off main, and sends again. An agent is the streaming half: its run computes off main and
streams events, and the agent's own `reduce` folds each event into the center's state on main, keyed by run.

## Swift to Rust

| Swift Granite | Here | Notes |
| --- | --- | --- |
| `MainActor`, `DispatchQueue.main`, `onMain` | `Main` | one thread, `granite.main`, draining turns in order; `Main` also owns the runtime effects and runs are spawned on |
| `GraniteState` | `GraniteState` | any `Clone + PartialEq + Default + Send + Sync` value |
| `GraniteCenter`, `GraniteCommand`, `Store` | `Center<S>` | the state in a `tokio::sync::watch`, so observers wake on commits; `commit` is main's and does nothing when the state is equal |
| `@Event` + `GraniteReducer` | `Event<R>` + `GraniteReducer` | a send is a deferred turn: clone, reduce, commit, then the effect; `debounced`, `throttled`, `lifecycle` |
| `GraniteEffect` (`.run`, `.chain`) | `Effect::{Run, Many}`, `Effect::chain` | spawned after the commit |
| `GraniteService`, `@Service`, `@Relay`, `Live.service` | `GraniteService`, `Relay::live()` | one live instance per type per process, made outside the registry's lock |
| `GraniteSignal`, `Prospect`, `GraniteCancellable` | `Signal`, `Scope`, `Subscription`, `Cancellable` | a dropped subscription ends; a scope's cancel is terminal |
| `GraniteTickets`, `GraniteLane`, `RunRegistry`, `TransientRegistry` | the same names, in `scaffolding` | a ticket's settle waits for main, so the write an effect sent first is in the state its waiter reads |
| `GraniteAgent`, `@Agent`, `AgentExecutable` | `GraniteAgent`, `Agent<A>` | `send`, `start`, `perform`, `wait`, `wait_prefix`, `cancel`, `cancel_prefix`, `events` |
| `AgentRunContainer` | `RunContainer<A>` (private) | ported line for line: supersede, drop, merge, parallel(max); a queued run keeps its id; events drained into one commit per turn; a superseded run's late events dropped; waits settle after the state is written |
| `AgentContext`, `ChildFrame`, `context.spawn` | `AgentContext`, `ChildFrame`, `ChildRun` | `deliver` (`yield` is a keyword), `note`, `hosted`; a child is cancelled with its parent |
| `AgentHost`, `AgentHostModel`, `AgentHostError` | the same names | in the kernel so a context can carry its host |

## How a run reaches the state

```text
agent.send(input)                       any thread
  └─ main: RunContainer::start          the policy: supersede, drop, merge, or queue for a lane
       └─ run task (runtime)            resolve compute → agent.run(input, context) → context.deliver(event) …
            └─ enqueue(event)           any thread; one drain scheduled per run
                 └─ main: drain         the run still current? coalesce, feed taps, reduce every event, commit once
                      └─ lifecycle      finished / failed / cancelled reduced and committed, then waiters settle
```

`perform` opens its wait and starts its run in one main turn, so no event lands between the two, and returns after the
lifecycle's commit. Dropping its future cancels a run it started or queued, never one it merged into.
