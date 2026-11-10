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
| `@Event` + `GraniteReducer` | `Event<R>` + `GraniteReducer` | a send is a deferred turn: clone, reduce, commit, then the effect; `debounced`, `throttled`, `lifecycle`; clones share one delivery |
| `GraniteDebounce`, `GraniteThrottle` | `Event::debounced`, `Event::throttled` | the timers hold the event weakly, so a payload still waiting goes with its last handle; a throttle sends the first at once, then one at the window's end (the latest, or the first) |
| `@Event(.onAppear)`, lifecycle hooks | `Center::on_lifecycle`, `Center::on_lifecycle_of`, `.lifecycle(when)` | hooks on an emitter; an event or agent hooked to its own center is held weakly, so whoever made it holds it |
| `GraniteEffect` (`.run`, `.chain`) | `Effect::{Run, Many}`, `Effect::chain` | spawned after the commit |
| `GraniteService`, `@Service`, `@Relay`, `Live.service` | `GraniteService`, `Relay::live()` | one live instance per type per process, made outside the registry's lock |
| `GraniteEmitter`, `GraniteEmitterKey` | `Emitter<T>`, `EmitterKey` | receivers held weakly and handed to their handler; a subscription ends with its receiver; a key replaces and moves last; delivery synchronous, in order, with the lock released |
| `GraniteLock<State>` | `Locked<T>` (private) | the body hands back what it takes out, dropped after the unlock; every kernel lock is one |
| `observe` → `AnyCancellable` | `Relay::observe` → `Observation` | the one token left: dropped, it ends the observation, and a delivery already queued does not run |
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

## Events

Granite's event layer keeps no tokens. An `Emitter` holds each receiver weakly and hands it to the handler on every
send, so a subscription lasts as long as whoever listens. Nothing is stored, cancelled, or torn down in a `Drop`, and
`off` is never needed for correctness.

```rust
let emitter = Emitter::<u32>::new();
let panel = Arc::new(Panel::default());
emitter.on(&panel, None, |panel, value| panel.show(*value)); // ends when `panel` is dropped
emitter.on(&panel, Some(EmitterKey::new("badge")), |panel, value| panel.badge(*value));
emitter.send(3);
```

- **Delivery** is synchronous, on the sender's thread, in subscription order, with the lock released: a handler may
  send, subscribe or unsubscribe on the same emitter. A receiver released earlier in the same send is not reached; an
  `off` or a replaced key takes effect from the next send.
- **A handler that captures its own receiver, or its own emitter, keeps it alive.** Reach the receiver through the
  argument it is handed.
- **Lifecycle hooks** ride a center's emitter. `on_lifecycle_of(&receiver, when, hook)` holds the receiver weakly;
  `Event::lifecycle` and `Agent::lifecycle` hook through it. Hold the event or agent: the center does not.
- **Timers** (`debounced`, `throttled`) hold their event weakly. A payload a timer holds back lives in the event and
  goes with its last handle.
- **Nothing is dropped under a lock.** Every kernel lock is a `Locked`. Whatever a locked body removes (an entry and its
  handler, a finished run and its compute, a replaced payload, a slot, a resident) is returned and dropped after the
  unlock, because its drop may run author code that reaches the same lock. `Locked::with` makes that the easy shape:
  in `x.lock().unwrap().remove(k);` the removed value is dropped before the guard, and in `with(|x| x.remove(k))` it
  is dropped after.

### Coming from signals

| Before | Now |
| --- | --- |
| `Signal<T>`, `observe` → `Subscription` | `Emitter<T>`, `on(&receiver, key, handler)`; it ends when the receiver is dropped |
| `Subscription::forget`, `store_in(&scope)` | hold the receiver: there is nothing to store |
| `Scope::cancel` | drop the receivers, or `Emitter::remove_all` |
| `Signal::seeded`, `last_value`, `observe_change` | keep the value in a center and read it, or `Center::watch` |
| `signal.debounced(d)`, `signal.throttled(d)` | `Event::debounced`, `Event::throttled` |
| `Relay::observe` → `Subscription` | `Relay::observe` → `Observation`; keep it as long as you observe |
