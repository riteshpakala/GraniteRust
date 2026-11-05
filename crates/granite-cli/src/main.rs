//! The `granite` command.
//!
//! ```text
//! granite agents add <name | git URL[@revision] | folder> --shelf <dir>
//! granite agents update <name> --shelf <dir>
//! granite agents rm <name> --shelf <dir>
//! granite agents list --shelf <dir> [--json]
//! granite describe <runner>
//! granite run <runner> <perform.json | -> [--directory <dir>] [--ask <text>] [--word <word>]
//! granite check --schemas <dir>… --schema <file or $id> <instance.json | ->
//! ```
//!
//! PIN: NO DEFAULT SHELF. A shelf is `--shelf` or `GRANITE_SHELF`, never a host's own folder by accident: Craft's
//! shelf is Craft's to write.
//! `run` is a host with no model: it prints one line, `{"output": …}` or `{"failed": …, "refused": …}`, with the
//! process's status, and exits 0 answered, 3 refused, 1 failed, 2 when the runner broke the wire or died.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use granite::Main;
use granite_agents::{
    AgentHost, AgentInstaller, AgentProcess, AgentRemoteError, AgentSource, AgentWire, json,
};
use serde_json::{Value, json};

const USAGE: &str = "usage:
  granite agents add <name | git URL[@revision] | folder> --shelf <dir>
  granite agents update <name> --shelf <dir>
  granite agents rm <name> --shelf <dir>
  granite agents list --shelf <dir> [--json]
  granite describe <runner>
  granite run <runner> <perform.json | -> [--directory <dir>] [--ask <text>] [--word <word>]
  granite check --schemas <dir>... --schema <file or $id> <instance.json | ->";

struct Arguments {
    words: Vec<String>,
    options: BTreeMap<String, Vec<String>>,
    flags: Vec<String>,
}

impl Arguments {
    fn parse(raw: Vec<String>) -> Result<Arguments, String> {
        let mut parsed = Arguments {
            words: Vec::new(),
            options: BTreeMap::new(),
            flags: Vec::new(),
        };
        let mut raw = raw.into_iter();
        while let Some(argument) = raw.next() {
            match argument.strip_prefix("--") {
                Some(flag @ ("json" | "yes")) => parsed.flags.push(flag.to_owned()),
                Some(option) => {
                    let value = raw
                        .next()
                        .ok_or_else(|| format!("--{option} needs a value"))?;
                    parsed
                        .options
                        .entry(option.to_owned())
                        .or_default()
                        .push(value);
                }
                None => parsed.words.push(argument),
            }
        }
        Ok(parsed)
    }

    fn one(&self, option: &str) -> Option<&str> {
        self.options
            .get(option)
            .and_then(|values| values.last())
            .map(String::as_str)
    }

    fn shelf(&self) -> Result<PathBuf, String> {
        self.one("shelf")
            .map(PathBuf::from)
            .or_else(|| std::env::var("GRANITE_SHELF").ok().map(PathBuf::from))
            .ok_or_else(|| "say which shelf: --shelf <dir> or GRANITE_SHELF".to_owned())
    }
}

fn read_input(said: &str) -> Result<Vec<u8>, String> {
    if said == "-" {
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        return Ok(bytes);
    }
    std::fs::read(said).map_err(|error| format!("{said}: {error}"))
}

fn main() -> ExitCode {
    let arguments = match Arguments::parse(std::env::args().skip(1).collect()) {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("{error}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let words: Vec<&str> = arguments.words.iter().map(String::as_str).collect();
    let result = Main::global().block_on(async {
        match words.as_slice() {
            ["agents", "add", source] => add(&arguments, source).await,
            ["agents", "update", name] => update(&arguments, name).await,
            ["agents", "rm", name] => remove(&arguments, name),
            ["agents", "list"] | ["agents"] => list(&arguments),
            ["describe", runner] => describe(runner).await,
            ["run", runner, perform] => run(&arguments, runner, perform).await,
            ["check", instance] => check(&arguments, instance),
            _ => Err((2, USAGE.to_owned())),
        }
    });
    match result {
        Ok(code) => ExitCode::from(code),
        Err((code, said)) => {
            eprintln!("granite: {said}");
            ExitCode::from(code)
        }
    }
}

type Outcome = Result<u8, (u8, String)>;

fn progress(progress: granite_agents::Progress) {
    match progress.line {
        Some(line) => eprintln!("  {line}"),
        None => eprintln!("{}", progress.stage),
    }
}

async fn add(arguments: &Arguments, source: &str) -> Outcome {
    let shelf = arguments.shelf().map_err(|said| (2, said))?;
    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let installer = AgentInstaller::new(&shelf);
    let installed = installer
        .install(AgentSource::parse(source, &base), progress)
        .await
        .map_err(|error| (1, error.to_string()))?;
    println!(
        "{}",
        String::from_utf8_lossy(&json::pretty(&installed).unwrap_or_default())
    );
    Ok(0)
}

async fn update(arguments: &Arguments, name: &str) -> Outcome {
    let installer = AgentInstaller::new(arguments.shelf().map_err(|said| (2, said))?);
    let installed = installer
        .update(name, progress)
        .await
        .map_err(|error| (1, error.to_string()))?;
    eprintln!(
        "updated {} at {}",
        installed.name,
        installed.revision.as_deref().unwrap_or("its folder")
    );
    Ok(0)
}

fn remove(arguments: &Arguments, name: &str) -> Outcome {
    let installer = AgentInstaller::new(arguments.shelf().map_err(|said| (2, said))?);
    let removed = installer
        .remove(name)
        .map_err(|error| (1, error.to_string()))?;
    eprintln!("removed {}", removed.name);
    Ok(0)
}

fn list(arguments: &Arguments) -> Outcome {
    let installer = AgentInstaller::new(arguments.shelf().map_err(|said| (2, said))?);
    let installations = installer.shelf.installations();
    if arguments.flags.iter().any(|flag| flag == "json") {
        println!(
            "{}",
            String::from_utf8_lossy(&json::pretty(&installations).unwrap_or_default())
        );
    } else {
        for installation in &installations {
            let at = installation
                .revision
                .as_deref()
                .map(|revision| &revision[..revision.len().min(7)])
                .unwrap_or("folder");
            for entry in &installation.entries {
                println!(
                    "{:<12} {}@{} · {}",
                    entry.word, installation.name, at, entry.summary
                );
            }
        }
        for problem in installer.shelf.problems() {
            eprintln!("{problem}");
        }
    }
    Ok(0)
}

async fn describe(runner: &str) -> Outcome {
    let entries = AgentProcess::new(runner)
        .describe()
        .await
        .map_err(|error| (2, error.to_string()))?;
    println!(
        "{}",
        String::from_utf8_lossy(&json::pretty(&entries).unwrap_or_default())
    );
    Ok(0)
}

async fn run(arguments: &Arguments, runner: &str, perform: &str) -> Outcome {
    let said: Value = serde_json::from_slice(&read_input(perform).map_err(|error| (2, error))?)
        .map_err(|error| (2, format!("{perform}: {error}")))?;
    let call = said.get("perform").cloned().unwrap_or(said);
    let word = arguments
        .one("word")
        .map(str::to_owned)
        .or_else(|| call["word"].as_str().map(str::to_owned));
    let Some(word) = word else {
        return Err((2, "say which agent: a perform's `word`, or --word".into()));
    };
    let input = call.get("input").cloned().unwrap_or(json!({}));
    let directory = arguments
        .one("directory")
        .map(str::to_owned)
        .or_else(|| call["directory"].as_str().map(str::to_owned));
    let directory = directory.map(|directory| {
        let path = Path::new(&directory);
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        };
        absolute.to_string_lossy().into_owned()
    });
    let ask = arguments
        .one("ask")
        .map(str::to_owned)
        .or_else(|| call["ask"].as_str().map(str::to_owned));
    let mut host = AgentHost::none().with_note(|line| eprintln!("note: {line}"));
    host.directory = directory;
    host.ask = ask;
    let status = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let kept = status.clone();
    let cost: granite_agents::process::CostFn = Box::new(move |cost| {
        eprintln!("{}", cost.sentence());
        *kept.lock().unwrap() = cost.status;
    });
    let input = serde_json::to_vec(&input).unwrap_or_default();
    let result = AgentProcess::new(runner)
        .perform(&word, &input, &host, Some(cost))
        .await;
    let status = status.lock().unwrap().clone();
    let (line, code) = match result {
        Ok(bytes) => {
            let output: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            (AgentWire::Output(output), 0)
        }
        Err(AgentRemoteError::Refused {
            refusal, message, ..
        }) => (
            AgentWire::Failed(granite_agents::Failure::refused(message, refusal)),
            3,
        ),
        Err(AgentRemoteError::Fault { message, .. } | AgentRemoteError::Failed { message, .. }) => {
            (AgentWire::Failed(granite_agents::Failure::said(message)), 1)
        }
        Err(other) => (
            AgentWire::Failed(granite_agents::Failure::said(other.to_string())),
            2,
        ),
    };
    let mut said: Value =
        serde_json::from_slice(&line.line().unwrap_or_default()).unwrap_or(Value::Null);
    said["status"] = Value::String(status);
    println!(
        "{}",
        String::from_utf8_lossy(&json::sorted(&said).unwrap_or_default())
    );
    Ok(code)
}

fn check(arguments: &Arguments, instance: &str) -> Outcome {
    let schema = arguments
        .one("schema")
        .ok_or((2, "say which schema: --schema <file or $id>".to_owned()))?;
    let mut options = jsonschema::options();
    let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
    for folder in arguments
        .options
        .get("schemas")
        .cloned()
        .unwrap_or_default()
    {
        let entries =
            std::fs::read_dir(&folder).map_err(|error| (2, format!("{folder}: {error}")))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.to_string_lossy().ends_with(".json") {
                continue;
            }
            let Ok(document) =
                serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap_or_default())
            else {
                continue;
            };
            let Some(id) = document["$id"].as_str().map(str::to_owned) else {
                continue;
            };
            by_id.insert(id.clone(), document.clone());
            by_id.insert(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                document.clone(),
            );
            let resource = jsonschema::Resource::from_contents(document)
                .map_err(|error| (2, format!("{}: {error}", path.display())))?;
            options.with_resource(id, resource);
        }
    }
    let document = match by_id.get(schema) {
        Some(document) => document.clone(),
        None => serde_json::from_slice(
            &std::fs::read(schema).map_err(|error| (2, format!("{schema}: {error}")))?,
        )
        .map_err(|error| (2, format!("{schema}: {error}")))?,
    };
    let validator = options
        .build(&document)
        .map_err(|error| (2, format!("{schema}: {error}")))?;
    let value: Value = serde_json::from_slice(&read_input(instance).map_err(|error| (2, error))?)
        .map_err(|error| (2, format!("{instance}: {error}")))?;
    let errors: Vec<String> = validator
        .iter_errors(&value)
        .map(|error| format!("{} at {}", error, error.instance_path))
        .collect();
    if errors.is_empty() {
        eprintln!("valid: {instance} against {schema}");
        return Ok(0);
    }
    for error in &errors {
        eprintln!("{error}");
    }
    Err((
        1,
        format!(
            "{instance} does not keep {schema}: {} problems",
            errors.len()
        ),
    ))
}
