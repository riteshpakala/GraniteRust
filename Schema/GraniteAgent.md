# The Granite agent

**`granite-agent/1`.** The standard for agents written on Granite in Swift: what an agent declares, how a host
finds it and calls it, and how one runs in a process of its own, on this machine or, later, another.

"Must" marks a rule a conforming agent or host keeps. Everything else describes. The machine-readable half is
generated from the code and checked against it:

| File | What it describes |
| --- | --- |
| [`granite-agent-entry.schema.json`](granite-agent-entry.schema.json) | One agent, described (§7) |
| [`granite-agent-wire.schema.json`](granite-agent-wire.schema.json) | Every message of the wire (§8) |
| [`granite-agent-installation.schema.json`](granite-agent-installation.schema.json) | An installed runner's record (§10) |
| [`granite-agent-answer.schema.json`](granite-agent-answer.schema.json) | A package call's answer, `granite-answer/1` (§4.2) |

## 1. Terms

- **Agent**: work that runs off the main thread and streams what it finds, so a person sees it as it goes.
- **Run**: one execution of an agent, for one input.
- **Package agent**: an agent shipped as a Swift package of its own and called by its word with JSON.
- **Worker spec**: a worker a host spawns on its own model, described by a package.
- **Host**: the app or process that calls an agent, and offers it a folder, an ask, notes and a model.
- **Call**: one request of a package agent: an input in; an output, or a failure, out.
- **Entry**: an agent described as data (§7).
- **Word**: an agent's name, as a host calls it and a model names it.
- **Runner**: an executable that carries package agents and answers calls on the wire (§8).
- **Shelf**: a folder of installed runners (§10).

## 2. The shapes of a Granite agent

### 2.1 An agent on a center

A `GraniteAgent` is declared on a Granite center beside its events, with `@Agent(concurrency) var name: Name.Agent`.
It declares an `Input`, an `Event` and a `Compute`, and two functions:

- `run(_:context:)` computes off main and streams events. It must not touch state. Children start through
  `context.spawn`, and `context.isCancelled` is its cooperative check.
- `reduce(state:event:run:)` folds each event into the center's state, on main, keyed by run. It is the only
  writer of what the run found.

`key(_:)` says which runs are the same, and `concurrency` says what a second send with a live key does:

| Concurrency | A second send while the first runs |
| --- | --- |
| `.supersede` | cancels the first and starts (the default) |
| `.drop` | is ignored |
| `.merge` | is handed to the live run (`merge`) |
| `.parallel(max:)` | runs beside it, up to `max` at once |

### 2.2 A package agent

A `GraniteAgentPackage` is a type in a package of its own:

```swift
public struct Marble: GraniteAgentPackage {
    public struct Input: Codable, Sendable { public var path: String; public var filter: MarbleFilter?; public var directory: String? }
    public typealias Event = MarbleFrame
    public typealias Output = MarbleResult
    public typealias Compute = MarbleCompute

    public static let word = "marble"
    public static let summary = "gives a photo a look from its catalog (sepia) on the GPU and writes the result beside it."
    public static let fields = ["path": "the photo to filter, relative to the project"]

    public static func compute() async throws -> MarbleCompute { try MarbleCompute() }
    public init() {}
    public func run(_ input: Input, context: Context) -> AsyncThrowingStream<MarbleFrame, Error> { … }
}
```

- It must declare `word`, `summary`, `Input` and `Output` (both `Codable`), and `run`.
- It may declare `fields`, `guide`, `lanes` (2 when left out), `makes` (§3) and `compute()`.
- Its `Output` should conform to `AgentAnswer` (§4.2): an output that does is checked and stamped before it
  leaves the process. One that does not answers as it always did, until the requirement lands (§11).
- `output(from:)` reads the answer off the run's events. When the output is the event type, it is the last event.
- A run must not touch state. Granite hosts each call on a center of its own and reduces the run's events there.

### 2.3 A worker spec

A `GraniteWorkerSpec` describes a worker the host runs on its own model: its `word` and `summary`, the host's
tools it may use by name (`toolNames`), a `budget` (`reading` or `shell`), whether it runs commands (`shell`),
an `opening` for its prompt, and a `reportRule`. It runs no code of its own.

### 2.4 The record

An agent is found, never listed. A package must place one record per agent in the binary:

```swift
@used @section("__DATA_CONST,__granite")
let marbleRecord: GraniteRoster.Record = { GraniteRoster.record(Marble.self) }   // record(worker:) for a spec
```

`GraniteRoster.discovered()` calls every record in every loaded image. A host or runner that links the package
finds its agents with nothing listed and nothing imported. A word is one agent: of two entries with one word,
the first found is kept.

## 3. Words and descriptions

- `word` must be lowercase, one word (`^[a-z][a-z0-9-]*$`), and unique among the agents a host runs.
- `summary` is one sentence of what the agent does. A host shows it to a model beside the tool.
- `guide` is how to use the agent well: what to ask of it, what to give it, what it is not for. A host puts
  it in front of the model when a person asks for the agent by name.
- `fields` holds a note per input field, shown beside the field.
- `makes` lists what the agent makes, one `AgentMake` per kind: `kind` (one lowercase word), `what` (what it is,
  in the words a reading can match to a request), `into` (where it writes, relative to the host's folder, `*` for
  one path component) and `never` (what it never does). A host reads a path under `into` as the agent's to write,
  and tells a model what each kind is. An answer's `made` and `expected` kinds are these kinds (§4.2).

```swift
static let makes = [AgentMake(kind: "view", what: "the social screens as reusable SwiftUI views; pages show them",
                              into: ["Shared/Views/Social/*"], never: "routes; never a page, a service or a model type")]
```

## 4. Input and output

### 4.1 Input

- An `Input` must be `Codable`. A host reads its schema off its own decoding (`GraniteSchema.of`):
  - A field read with `decodeIfPresent` is optional, so a field a call may leave out must be optional. A
    default value in a synthesized `init(from:)` still decodes as required.
  - A `String` enum that is `CaseIterable` is a choice of its cases.
  - A field named `directory` is the host's (§6). A host must fill it, and must not show it to a model.
- `compute()` is made once per process, on the first call; a process that never calls makes none.
- `lanes` caps how many calls of the agent run at once.

### 4.2 The answer: `granite-answer/1`

An `Output` must be `Codable`. Files the call wrote must be named in `path` or `paths`, as absolute paths or
relative to `directory`; a host counts them as the call's changes. An `Output` that conforms to `AgentAnswer`
answers `granite-answer/1`: one JSON object whose reserved keys a host reads, beside the package's own, which a
host passes on without reading.

```json
{"format": "granite-answer/1", "agent": {"word": "loom"},
 "path": "/p/Shared/Views/Social/FeedView.swift",
 "paths": ["/p/Shared/Views/Social/FeedView.swift", "/p/Shared/Views/Social/Conformances/Post+Post.swift"],
 "expected": [{"kind": "view", "name": "FeedView"}, {"kind": "conformance", "name": "Post"}],
 "made": [{"kind": "view", "name": "FeedView", "path": "/p/Shared/Views/Social/FeedView.swift", "state": "written"},
          {"kind": "conformance", "name": "Post", "path": "/p/Shared/Views/Social/Conformances/Post+Post.swift", "state": "rewritten"}],
 "problems": [{"kind": "skipped", "text": "edited=nil left out: a property a line leaves out shows its default"}],
 "notes": ["`Post` is declared in MarginKit, so the conformances import it."],
 "views": ["FeedView"], "schema": "AppSocial"}
```

| Key | Who writes it | What it holds |
| --- | --- | --- |
| `format` | the runner | `granite-answer/1` |
| `agent` | the runner | `{"word": …}` |
| `path` | the package | the first file it wrote, or null |
| `paths` | the package | every file it wrote or removed; `[]` when none |
| `notes` | the package | what the model should know, one sentence each |
| `made` | the package | what it made: `kind` (one of its `makes`), `name`, `path`, `state` (`written`, `rewritten`, `unchanged`, `removed`) |
| `expected` | the package | what its call asked for, as it read the call: `kind`, `name`, the measure `made` is held to |
| `problems` | the package | what it could only do in part (a fallback, a model that said nothing): `kind`, `text`; not a refusal |
| `payloads` | the package | files among `paths` that follow a standard of their own: `path`, and `schema`, the id the file's own `schema` key says (`craft-timeseries/1`) |

A runner checks every conforming answer before it is sent (`AgentAnswerCheck`), and refuses it as kind `answer`
(§4.3) when:

- `format` or `agent` is in it (they are the runner's to write);
- `paths` is missing, or `path` is not one of `paths`;
- a `made` or `expected` kind is not one the package declares in `makes`, or a `made` item lacks its `kind` or
  `name`, or a written or rewritten one names no `path` among `paths`;
- a payload is not among `paths`, is not a JSON object on disk, or its own `schema` is not the one named;
- `problems` or `notes` is not a list of its shape.

Paths are compared as files: each read against `directory` when relative and standardized. The key a host reads
for measure is `made` against `expected`: what the agent was told, and what it made of it.

### 4.3 Refusals

Every way a call is not answered is one shape, `AgentRefusal`: a `kind` and `reasons`, one sentence each with its
fix in it. A package throws one, or an error of its own type that is `AgentRefusing`:

| Kind | When |
| --- | --- |
| `input` | the call's JSON does not read as the input |
| `call` | the call cannot be honoured as written: the fix is the caller's |
| `build` | a build the call needs failed |
| `device` | a device or simulator the call needs failed |
| `model` | the host's model gave nothing usable |
| `parse` | Swift the package wrote did not parse, so nothing was written |
| `reach` | a service it reads could not be reached: try later |
| `answer` | its answer broke `granite-answer/1` (§4.2): the package's own bug |
| `host` | the host could not serve it |

An error that is neither is the package's fault, not a refusal: a host says it as it is and does not ask the
caller to correct it.

## 5. The input schema as data

A schema crosses a process, or sits in a record, in this form. It reads back exactly, field order and `any`
included:

```json
{"object": [
  {"name": "path", "required": true, "schema": "string"},
  {"name": "filter", "required": false, "schema": {"choice": ["sepia"]}},
  {"name": "sizes", "required": false, "schema": {"array": "integer"}},
  {"name": "extra", "required": false, "schema": "any"}
]}
```

A scalar is a word: `string`, `integer`, `number`, `boolean` or `any`. Anything else is exactly one key:
`choice` (the values), `array` (the element's schema) or `object` (the fields, in order). A host shows a model
JSON Schema instead, from `jsonSchema(notes:hiding:)`, which is not this form and does not read back.

## 6. What a host offers

A run reads what its host offers through `context.host`, an `AgentHost`:

| | What it is | When absent |
| --- | --- | --- |
| `directory` | The folder the host works in | nil: the call has no folder |
| `ask` | What the person asked, in their words | nil: no person asked (a script, a test) |
| `note(_:)` | A line of progress the host shows as it shows its agents' work | Does nothing |
| `model` | The model the host has loaded (`AgentHostModel`) | nil: the host has none |

- `model.object(schema:system:user:maxTokens:seconds:)` answers a JSON object that fits a JSON Schema.
- `model.text(system:user:maxTokens:seconds:)` answers text.
- An agent that cannot work without the model must ask `context.host.model(for: word)`. It throws
  `AgentHostError.noModel`, a sentence naming the agent, when there is none.
- A failed ask throws `AgentHostError.noAnswer`. An answer that does not decode throws `.answer`.
- The ask is never a field of `Input`. A model writes a call's input; it never writes what the person asked.

## 7. The entry

An entry is one agent described as data: everything a host needs to offer the agent and call it without the
type that declares it. `--describe` prints entries (§8), an installation keeps them (§10), and the entry schema
holds them.

```json
{"kind": "package", "word": "marble", "summary": "gives a photo a look from its catalog (sepia) on the GPU …",
 "module": "MarbleAgent", "type": "MarbleAgent.Marble", "schema": {"object": […]},
 "fields": {"path": "the photo to filter, relative to the project"}, "lanes": 2, "guide": "…"}
```

- Every entry has `kind` (`package` or `worker`), `word`, `summary`, `module` and `type`.
- A package's entry adds `schema`, `fields`, `lanes`, `guide` and `makes` (§3).
- A worker's entry adds `toolNames`, `budget`, `shell` and `reportRule`. A worker described by an entry opens
  its prompt as its host's workers do.

## 8. The wire

A runner and its host speak `granite-agent/1`: JSON objects, one per line (UTF-8, ending `\n`). Each names
itself by its one key. The runner writes to its stdout and reads its stdin. Its stderr is its log.

- **Stdout is the wire's alone.** Nothing else may write to it. A runner must move the real stdout aside
  before any package runs (`AgentServer.main()` does), so a `print` lands in the log.
- **One call per process.** A runner answers one `perform` with exactly one `output` or `failed`, then exits.

| Message | From | When | Body |
| --- | --- | --- | --- |
| `hello` | runner | first | `wire` (`"granite-agent/1"`), `pid`, `answer` (`"granite-answer/1"` from a runner whose calls keep it) |
| `perform` | host | after `hello` | `word`, `input`, `directory`, `ask`, `model` (whether the host offers one) |
| `note` | runner | any time | a line of progress |
| `model` | runner | any time | `id`, `kind` (`object` or `text`), `schema`, `system`, `user`, `maxTokens`, `seconds` |
| `answer` | host | after a `model` | `id`, and exactly one of `object`, `text` or `error` |
| `output` | runner | last | the call's answer (§4.2), or a package's own JSON |
| `failed` | runner | last | a sentence: why the call failed; beside it, `refused` (§4.3) when it was refused |
| `cancel` | host | any time | `{}`: end the call |
| `entries` | runner | with `--describe` only | the entries it carries (§7) |

A call, line by line:

```
← {"hello":{"pid":4512,"wire":"granite-agent/1"}}
→ {"perform":{"ask":"make a graphic of the flow","directory":"/p","input":{"sources":["Sources"]},"model":true,"word":"plate"}}
← {"note":"planning the plate from 11 parts and 11 references"}
← {"model":{"id":1,"kind":"object","maxTokens":3000,"schema":{…},"seconds":300,"system":"…","user":"…"}}
→ {"answer":{"id":1,"object":{…}}}
← {"note":"round 1: measured clean"}
← {"output":{"agent":{"word":"plate"},"format":"granite-answer/1","made":[{"kind":"plate","name":"one-turn-every-service","path":"/p/docs/one-turn-every-service.png","state":"written"}],"path":"/p/docs/one-turn-every-service.png","paths":["/p/docs/one-turn-every-service.png"],"width":3200}}
```

Further rules:

- **Answers match by `id`.** Asks may overlap, and an answer may come in any order.
- **Ending a call.** `cancel`, or SIGTERM to the runner's process group, ends the run, and the runner says
  `failed`. A host that receives no answer must end the process: SIGTERM, then SIGKILL after a grace.
- **The host's end closing is not a cancel.** It means the host sends nothing more. The call runs on, and an
  ask of the model is told there is no answer.
- **Exit codes:**
  - `0`: answered.
  - `1`: failed.
  - `2`: the host said something that is not a call, or left before making one.
  - `3`: refused.
  - `130`: cancelled.
- **A refusal or a fault.** From a runner whose `hello` names `answer`, a `failed` line with `refused` is a
  refusal and one without is the agent's fault. From a runner that names none, a host cannot tell them apart.
- **The host counts the asks.** Every `model` message is one ask of the host's model; a host counts them per
  call and reports them with what the process took (`AgentProcessCost.asks`).
- **Unknown fields are ignored; unknown messages are not.** A reader must ignore a field it does not know. A
  message it does not know ends the call as a fault of the wire.
- **The runner's main is never blocked.** Granite reduces on main, so the runner's `main` must be async and
  only await, and the wire must be read on a thread of its own.

A runner is one file in an executable that links the packages and imports none of them:

```swift
import GraniteAgents

@main
enum Agent {
    static func main() async { await AgentServer.main() }
}
```

`agent --describe` writes one `entries` line and exits, making no compute.

## 9. Lifecycle

- **One process, one call, then released.** A host spawns a runner for a call, and the process exits when it
  has answered. What the call made goes with it: a GPU device, a web page, a model's asks. Between calls,
  nothing of the agent is resident.
- **Each call starts fresh.** `compute()` is made in the call's process.
- **A host measures what the process took.** It reaps the process and reports wall time, CPU time and peak
  resident memory: "released · peak 412 MB · 1.8 s CPU in 2.4 s". That is the measure a scheduler places a call
  by.
- **Lanes cap processes.** A host runs at most `lanes` processes of one agent at once.

## 10. Installation

A host installs a package as a runner on its shelf:

1. **Fetch.** A repository is cloned at a revision, or its default branch's head. A folder is used as it is.
2. **Read the manifest.** `swift package dump-package` gives the package's name, its libraries, the macOS it
   needs, and its Granite requirement. A package that does not depend on Granite is refused.
3. **Write the runner.** One executable target, `agent`, depends on every library of the package and on
   `GraniteAgents` at the package's own requirement. Its one file is the runner of §8.
4. **Build** it in release.
5. **Describe and check.** `agent --describe` gives its entries. A runner with none, or with a word another
   installation carries, is refused.
6. **Install** `agent` and every resource bundle beside it into `<shelf>/<Name>/`, with `installation.json`,
   whole or not at all.

```json
{"format": "granite-agent-installation/1", "name": "PlateAgent",
 "source": {"url": "https://github.com/riteshpakala/PlateAgent.git"}, "revision": "f9bdda72…",
 "granite": "e64723f3…", "executable": "agent", "sha256": "…", "size": 9123456, "modified": 1793427000.5,
 "builtAt": "2026-10-31T08:00:00Z", "entries": [ … ]}
```

- **Tamper rule.** A shelf is a folder anything running as the user can write to. A host must check the
  executable's SHA-256 against the record before a launch, whenever its `size` or `modified` changed. It must
  not run one that differs; installing again is the way back.
- **Offering an installation.** A host offers a shelf's agents from the records alone, without running a
  runner. It reads the shelf again when a record changes.

## 11. Versioning

- **The version.** `granite-agent/<major>` names the wire, the entry and the installation together.
- **A minor change** adds an optional field. A reader must ignore fields it does not know, so it stays
  `granite-agent/1`. `makes`, `hello.answer`, `refused` and the answer's reserved keys came so.
- **The answer.** `granite-answer/<major>` names the answer (§4.2) apart from the wire: a new reserved key is
  minor. Conforming is opt-in until every agent of a shelf conforms; then `Output: AgentAnswer` is required,
  in a release of its own, so no install of an older agent stops building in between.
- **A breaking change** takes the next major. A host must refuse a runner whose `hello` names a major it does
  not speak, with a sentence.
- **Records.** An installation's `format` names its record's version, and a host skips a record it cannot read.

## 12. Checklists

**A package author:**

- [ ] The package depends on `GraniteAgents` and imports nothing else of Granite's it does not use.
- [ ] Each agent declares a lowercase `word`, a one-sentence `summary`, `Codable` `Input` and `Output`, and a
      `run` that touches no state.
- [ ] A field a call may leave out is optional; choices are `CaseIterable` `String` enums; `directory` is the
      host's.
- [ ] Files written are named in `path` or `paths`.
- [ ] `makes` says each kind it makes and where; its `Output` is an `AgentAnswer` that says `made` against
      `expected`, and its errors are `AgentRefusal`s or `AgentRefusing`.
- [ ] Compute is made in `compute()`; the model is asked through `context.host.model(for:)`.
- [ ] `guide` says how to use it well when `summary` cannot.
- [ ] Each agent has its two-line record, and a test finds it with `GraniteRoster.discovered()`.
- [ ] Nothing prints for a host to read: notes go through `context.host.note`.

**A host:**

- [ ] It offers what it has through `AgentHost`, and fills `directory`.
- [ ] It runs a runner per call, speaks `granite-agent/1`, matches answers by id, and ends a process that
      does not answer.
- [ ] It reaps each process, and reports what it took, its asks counted.
- [ ] It reads a refusal by its kind, and a fault as a fault.
- [ ] It installs from source (§10), refuses clashing words, and checks the digest before every launch.
- [ ] It offers a shelf from its records, and reads it again when they change.
