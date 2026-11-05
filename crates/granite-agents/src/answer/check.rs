//! An answer held to `granite-answer/1` before it leaves the process: Swift Granite's `AgentAnswerCheck`,
//! rule for rule and sentence for sentence.
//!
//! PIN: PATHS COMPARED AS FILES. Each path is read against the host's folder (a relative one) and resolved on
//! both sides, so an answer that says `paths` relative and `made` absolute is not refused for it.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value};

use super::{AnswerStandard, MadeState};
use crate::AgentMake;

pub struct AnswerCheck;

fn string(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

fn lowercase_word(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl AnswerCheck {
    /// Every way `answer` breaks the standard, one sentence each; empty when it keeps it.
    pub fn problems(
        answer: &Map<String, Value>,
        word: &str,
        makes: &[AgentMake],
        directory: Option<&str>,
    ) -> Vec<String> {
        let mut problems = Vec::new();
        for key in AnswerStandard::STAMPED {
            if answer.contains_key(key) {
                problems.push(format!(
                    "`{key}` is the runner's to say: an answer does not encode it"
                ));
            }
        }
        let mut paths = BTreeSet::new();
        match answer.get("paths") {
            Some(Value::Array(items)) => {
                for item in items {
                    match item {
                        Value::String(path) => {
                            paths.insert(Self::resolved(path, directory));
                        }
                        _ => problems.push("`paths` holds something that is not a path".into()),
                    }
                }
            }
            None => problems.push(
                "`paths` is missing: an answer names every file it wrote, `[]` when none".into(),
            ),
            Some(_) => problems.push("`paths` is not a list of paths".into()),
        }
        match answer.get("path") {
            None | Some(Value::Null) => {}
            Some(Value::String(path)) => {
                if !paths.contains(&Self::resolved(path, directory)) {
                    problems.push(format!("`path` {path} is not one of `paths`"));
                }
            }
            Some(_) => problems.push("`path` is not a path".into()),
        }
        let kinds: BTreeSet<&str> = makes.iter().map(|make| make.kind.as_str()).collect();
        let declared = if makes.is_empty() {
            "declares nothing it makes (`makes`)".to_owned()
        } else {
            format!(
                "makes {}",
                makes
                    .iter()
                    .map(|make| format!("`{}`", make.kind))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        problems.extend(Self::list(answer.get("made"), "made", |item, index| {
            let (Some(kind), Some(name)) = (string(item.get("kind")), string(item.get("name")))
            else {
                return vec![format!("`made[{index}]` needs a `kind` and a `name`")];
            };
            if name.is_empty() {
                return vec![format!("`made[{index}]` needs a `kind` and a `name`")];
            }
            let mut said = Vec::new();
            if !kinds.contains(kind) {
                said.push(format!(
                    "`made[{index}]` is a `{kind}`, and {word} {declared}"
                ));
            }
            let state = string(item.get("state")).and_then(|state| {
                MadeState::ALL
                    .into_iter()
                    .find(|known| known.as_str() == state)
            });
            if item.contains_key("state") && state.is_none() {
                let states: Vec<String> = MadeState::ALL
                    .iter()
                    .map(|state| format!("`{}`", state.as_str()))
                    .collect();
                said.push(format!(
                    "`made[{index}].state` is one of {}",
                    states.join(", ")
                ));
            }
            if matches!(
                state.unwrap_or(MadeState::Written),
                MadeState::Written | MadeState::Rewritten
            ) {
                match string(item.get("path")) {
                    Some(path) => {
                        if !paths.contains(&Self::resolved(path, directory)) {
                            said.push(format!(
                                "`made[{index}]` is in {path}, which is not one of `paths`"
                            ));
                        }
                    }
                    None => said.push(format!(
                        "`made[{index}]` was written, so it names its `path`"
                    )),
                }
            }
            said
        }));
        problems.extend(Self::list(
            answer.get("expected"),
            "expected",
            |item, index| {
                let (Some(kind), Some(_)) = (string(item.get("kind")), string(item.get("name")))
                else {
                    return vec![format!("`expected[{index}]` needs a `kind` and a `name`")];
                };
                if kinds.contains(kind) {
                    vec![]
                } else {
                    vec![format!(
                        "`expected[{index}]` is a `{kind}`, and {word} {declared}"
                    )]
                }
            },
        ));
        problems.extend(Self::list(
            answer.get("problems"),
            "problems",
            |item, index| {
                let (Some(kind), Some(_)) = (string(item.get("kind")), string(item.get("text")))
                else {
                    return vec![format!("`problems[{index}]` needs a `kind` and a `text`")];
                };
                if lowercase_word(kind) {
                    vec![]
                } else {
                    vec![format!("`problems[{index}].kind` is one lowercase word")]
                }
            },
        ));
        problems.extend(Self::list(answer.get("payloads"), "payloads", |item, index| {
            let (Some(path), Some(schema)) = (string(item.get("path")), string(item.get("schema"))) else {
                return vec![format!("`payloads[{index}]` needs a `path` and a `schema`")];
            };
            let file = Self::resolved(path, directory);
            if !paths.contains(&file) {
                return vec![format!("`payloads[{index}]` {path} is not one of `paths`")];
            }
            let Some(Value::Object(payload)) = std::fs::read(&file).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()) else {
                return vec![format!("`payloads[{index}]` {path} is not a JSON object on disk")];
            };
            match string(payload.get("schema")) {
                Some(said) if said == schema => vec![],
                Some(said) => vec![format!("`payloads[{index}]` says `{schema}`, and the file's own `schema` is `{said}`")],
                None => vec![format!("`payloads[{index}]` says `{schema}`, and the file's own `schema` is missing")],
            }
        }));
        match answer.get("notes") {
            None => {}
            Some(Value::Array(items)) if items.iter().all(Value::is_string) => {}
            Some(_) => problems.push("`notes` is a list of sentences".into()),
        }
        problems
    }

    /// The answer with the standard's id and the agent's word written in.
    pub fn stamped(answer: &Map<String, Value>, word: &str) -> Map<String, Value> {
        let mut out = answer.clone();
        out.insert(
            "format".into(),
            Value::String(AnswerStandard::FORMAT.into()),
        );
        let mut agent = Map::new();
        agent.insert("word".into(), Value::String(word.into()));
        out.insert("agent".into(), Value::Object(agent));
        out
    }

    /// A path as a file: under `directory` when relative (the process's folder when there is none), `.` and `..`
    /// collapsed, and the symlinks of its longest existing ancestor resolved (`/tmp` is `/private/tmp` on macOS).
    pub fn resolved(path: &str, directory: Option<&str>) -> String {
        let joined = if path.starts_with('/') {
            PathBuf::from(path)
        } else {
            let base = directory
                .map(PathBuf::from)
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("/"));
            base.join(path)
        };
        let mut lexical = PathBuf::from("/");
        for component in joined.components() {
            match component {
                Component::ParentDir => {
                    lexical.pop();
                }
                Component::Normal(part) => lexical.push(part),
                _ => {}
            }
        }
        let mut existing: &Path = &lexical;
        let mut rest = Vec::new();
        loop {
            if let Ok(real) = existing.canonicalize() {
                let mut out = real;
                for part in rest.iter().rev() {
                    out.push(part);
                }
                return out.to_string_lossy().into_owned();
            }
            match (existing.parent(), existing.file_name()) {
                (Some(parent), Some(name)) => {
                    rest.push(name.to_owned());
                    existing = parent;
                }
                _ => return lexical.to_string_lossy().into_owned(),
            }
        }
    }

    /// Each item of a reserved list checked by `each`; a key that is not a list of objects is one problem.
    fn list(
        value: Option<&Value>,
        key: &str,
        each: impl Fn(&Map<String, Value>, usize) -> Vec<String>,
    ) -> Vec<String> {
        match value {
            None => vec![],
            Some(Value::Array(items)) => items
                .iter()
                .enumerate()
                .flat_map(|(index, item)| match item {
                    Value::Object(object) => each(object, index),
                    _ => vec![format!("`{key}[{index}]` is not an object")],
                })
                .collect(),
            Some(_) => vec![format!("`{key}` is not a list")],
        }
    }
}
