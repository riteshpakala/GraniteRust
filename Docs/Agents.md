# Writing an agent package

An agent package is a Cargo repository whose runner bin hands its packages to `AgentServer::main`. The standard it keeps
is `Schema/GraniteAgent.md`; this page is how to keep it in Rust. The README has the smallest package whole.

## The repository

- A crate that declares one or more `AgentPackage`s, depending on `granite-agents` by git URL (`branch = "main"`); its
  `Cargo.lock` is committed, so an install builds the GraniteRust commit it was tested on.
- A bin whose `main` is `AgentServer::main(roster![…])`, named in `[package.metadata.granite] agent = "<bin>"`
  (or the only bin, or a bin called `agent`). `name = "<Name>"` there names the installation; the repository's folder
  name otherwise. In a workspace, the member with that table is the one installed.
- `Schema/` with the agent's copy of the standard (Craft writes it: `craft agents schema <word> --into Schema`), and
  `Schema/examples/` with calls the container check makes (`*.perform.json`, `*.refusal.json`).

## The package

| Item | What it says |
| --- | --- |
| `WORD` | how a host calls it and the model's tool name: lowercase, `^[a-z][a-z0-9-]*$` |
| `SUMMARY` | what it does, one sentence a model sees beside the tool |
| `fields()` | a note per input field |
| `guide()` | how to use it well: what a host puts in front of its model when a person asks for the agent by name |
| `lanes()` | how many calls of it run at once; two unless it says |
| `makes()` | `AgentMake { kind, what, into, never }`: what it writes and where; an answer's `made` and `expected` kinds are these |
| `module()` | the name a host also resolves it by (`$ComicAgent`); its crate's name unless it says one |
| `schema()` | what its input takes; read off the input's own `Deserialize` unless it says one |
| `compute()` | what every run of the process shares, made on the first call (a GPU device), never at `--describe` |
| `run(input, context)` | the work: `context.deliver(event)` streams, `context.note(line)` is progress the host shows, `context.host` is the folder, the person's ask and the host's model |
| `output(events)` | the answer, read off the events |

**The input's schema is read, not written.** `GraniteSchema::of::<Input>()` runs the input's own `Deserialize` once
against a recorder: an `Option` field is optional, any other field required, a unit-variant enum a choice of its
serialized names, fields in declaration order. A field named `directory` is the host's to fill and is never shown to a
model. Write `#[serde(default)]` fields as `Option` instead (a default still reads as required, as in Swift), and say a
hand-written `Deserialize`'s schema in `schema()`.

**The answer is `granite-answer/1`, always.** `Output: AgentAnswer`; `Answer<T>` is a ready one with the package's own
keys in `own`. Name every file written in `path`/`paths`, what was made in `made` (a declared kind, a name, the path,
`written`/`rewritten`), what the call asked for in `expected`, what could only be done in part in `problems`. The
runner checks it (`AnswerCheck`) and stamps `format` and `agent` before it leaves the process, and refuses one that
breaks the standard as kind `answer`.

**A refusal is returned as one.** `Err(Box::new(AgentRefusal::one(RefusalKind::Call, "…: give …")))`, or an error type
of the package's own that implements `AgentRefusing`, returned as `Err(error.refused())`. The kinds: `input`, `call`
(the caller's to correct), `build`, `device`, `model`, `reach`, `host` (what stood in the way), `parse`, `answer` (the
package's own bug). Any other error is a fault, said as it is and never retried.

**The host's model** is `context.host.model_for(WORD)?` (a host without one refuses the call as `model`), asked with
`host_ext::object_value(model, &json_schema, system, user, max_tokens, seconds)` or `object_as::<T>`.

**Stdout is the wire's.** The runner moves the real stdout aside before any package runs; a `println!` lands on stderr,
which the host keeps the end of for a crash's message.

## Testing one

- In process: `AnyAgentPackage::of::<P>().perform(input_bytes, host).await` gives the stamped answer, or the refusal
  as an `AgentPackageError`; a host model is any `AgentHostModel`.
- Over the wire: `AgentProcess::new(env!("CARGO_BIN_EXE_<bin>"))`, `describe()` and `perform(…)`.
- Installed: `containers/check.sh <repository>` (see Containers).
