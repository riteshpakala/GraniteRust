# Hosting runners

A host calls an agent by running its runner, one process per call, and offers it what it has: the folder it works in,
the person's ask, a line of progress it shows, and its model.

## One call

```rust
let process = AgentProcess::new("/shelf/ComicAgent/agent");
let entries = process.describe().await?;                       // `--describe`: nothing is made to say it
let host = AgentHost::none().with_directory("/project").with_ask("explain recursion").with_model(model).with_note(|line| println!("{line}"));
let answer = process.perform("comic", br#"{"brief": "recursion"}"#, &host, Some(Box::new(|cost| println!("{}", cost.sentence())))).await?;
```

- The process starts in its own process group, in the call's folder, with the host's environment and `GRANITE_AGENT=1`.
- `hello` must name `granite-agent/1`; then `perform`; then notes, asks of the model (answered by the host, counted),
  and one `output` or `failed`.
- `failed` with `refused` is `AgentRemoteError::Refused` (the kind is the model's to read); without one, from a runner
  whose hello named `granite-answer/1`, it is the agent's `Fault`.
- Dropping the call sends `cancel`, then SIGTERM to the group and SIGKILL two seconds later. A process still there
  five seconds after it answered is ended the same way.
- The process is reaped with `wait4`: wall time, CPU time, peak memory, how it ended, the asks: "released · peak
  61 MB · 0.1 s CPU in 0.1 s".

## The shelf

A shelf is a folder of installations, `<shelf>/<Name>/{agent, installation.json}`. `AgentShelf::entries` offers every
agent on it from the records alone; each call checks the executable first (its size and modification time, else its
SHA-256) and refuses one that changed since it was installed. `stamp()` changes whenever an installation does.

## The installer

`AgentInstaller::new(shelf).install(source, progress)`, a source being a git URL (`@revision` optional) or a folder:

1. **fetch**: `git clone`, `git checkout <revision>`, `git rev-parse HEAD`;
2. **reading the package**: `cargo metadata`, the package that names its runner, and the `granite-agents` it resolves
   (`version@commit`, the installation's `granite`);
3. **building**: `cargo build --release -p <package> --bin <runner>` into a target the shelf's installs share;
4. **asking what it carries**: `<runner> --describe`; none, or a word another installation carries, is refused;
5. **installing**: the runner copied as `agent`, `installation.json` written, the folder moved into place whole.

The progress stages are Swift Granite's words, so a host that shows a Swift install shows this one the same way.
