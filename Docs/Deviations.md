# Deviations from the Swift code

Each is deliberate. Where the Swift code and `Schema/GraniteAgent.md` disagree, this port follows the standard.

1. **Input that does not read is a refusal.** Kind `input`, exit 3, as §4.3 says. Swift's runner says it as a plain
   `failed` (exit 1), which a host reads as the agent's fault.
2. **Every answer is `granite-answer/1`.** `Output: AgentAnswer` is required, and `hello` always names the answer
   standard: §11's end state, since no Rust agent predates it.
3. **The roster is listed.** A runner's `main` lists its packages (`roster![…]`); there is no `__DATA_CONST,__granite`
   section. Hosts never read a binary's sections: they find agents by `--describe` and `installation.json`.
4. **The runner is the package's.** The installer builds the bin the package names
   (`[package.metadata.granite] agent`), rather than generating one.
5. **Reducers are synchronous and commit on main.** No `.task`, `.streamingTask` or `.persistentStreamingTask`
   reducers; the agent engine is the streaming path. A send always commits one main turn later.
6. **No reflection.** Events are made (`Event::new(center, reducer)`), not discovered by a compile pass; no
   `Forwarding`, no `@Notify`, no `GraniteCustomEvent`.
7. **No ambient scope.** Subscriptions are held or stored in a `Scope`; dropping one ends it.
8. **`TransientState` is not a type.** A center without persistence is transient.
9. **`RunScope` does not reach nested spawns.** A `tokio::spawn` inside a run does not inherit its scope, where Swift's
   `Task {}` inherits task-locals.
10. **A run's cancel is an abort.** Cancelling drops the run's future at its next await (and sets its flag for work
    on other threads); Swift's is cooperative. Cleanup belongs in `Drop`.
11. **Nothing of SwiftUI.** Components, navigation, storage and the debug overlay are not ported.
