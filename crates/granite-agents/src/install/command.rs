//! A tool an install runs (`git`, `cargo`): its output kept whole, each line said as it comes.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use super::AgentInstallError;

pub(crate) struct Said {
    pub status: i32,
    pub stdout: String,
    /// stdout and stderr, interleaved as they came.
    pub output: String,
}

impl Said {
    pub fn ok(&self) -> bool {
        self.status == 0
    }
}

pub(crate) type Line = Box<dyn Fn(&str) + Send + Sync>;

/// Runs `arguments` in `folder` with no stdin and the dynamic loader's variables left out.
pub(crate) async fn run(
    arguments: &[&str],
    folder: &Path,
    line: Option<Line>,
) -> Result<Said, AgentInstallError> {
    let mut command = Command::new(arguments[0]);
    command
        .args(&arguments[1..])
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, _) in std::env::vars() {
        if key.starts_with("DYLD_") {
            command.env_remove(key);
        }
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Said {
                status: 127,
                stdout: String::new(),
                output: format!("{} was not found", arguments[0]),
            });
        }
        Err(error) => {
            return Err(AgentInstallError::Toolchain(format!(
                "{} could not start: {error}",
                arguments[0]
            )));
        }
    };
    let output = Arc::new(Mutex::new(String::new()));
    let line: Arc<Option<Line>> = Arc::new(line);
    let mut stdout_lines = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
    let mut stderr_lines = BufReader::new(child.stderr.take().expect("piped stderr")).lines();
    let (out, said) = (output.clone(), line.clone());
    let errors = tokio::spawn(async move {
        while let Ok(Some(text)) = stderr_lines.next_line().await {
            out.lock().unwrap().push_str(&format!("{text}\n"));
            if let Some(said) = said.as_ref() {
                said(&text)
            }
        }
    });
    let mut stdout = String::new();
    while let Ok(Some(text)) = stdout_lines.next_line().await {
        stdout.push_str(&text);
        stdout.push('\n');
        output.lock().unwrap().push_str(&format!("{text}\n"));
        if let Some(said) = line.as_ref() {
            said(&text)
        }
    }
    let _ = errors.await;
    let status = child
        .wait()
        .await
        .map(|status| status.code().unwrap_or(1))
        .unwrap_or(1);
    let output = output.lock().unwrap().clone();
    Ok(Said {
        status,
        stdout,
        output,
    })
}
