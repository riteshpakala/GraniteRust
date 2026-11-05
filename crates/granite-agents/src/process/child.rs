//! A runner's process: spawned in a group of its own with three pipes, its stderr's end kept, reaped by `wait4`
//! for what it took.
//!
//! PIN: NEVER `Child::wait`. The process is reaped once, by a thread of its own, with `wait4`, so its usage is
//! measured; std never reaps it.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::watch;

use super::{AgentProcessCost, AgentRemoteError};
use crate::PipeChannel;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Ended {
    pub status: i32,
    pub cpu_seconds: f64,
    pub peak_bytes: i64,
}

pub(crate) struct AgentChild {
    pid: i32,
    pub channel: Arc<PipeChannel>,
    started: Instant,
    stderr: Arc<Mutex<Vec<u8>>>,
    ended: watch::Receiver<Option<Ended>>,
}

/// `exit N` or `signal N`.
pub(crate) fn describe(raw: i32) -> String {
    let signal = raw & 0x7f;
    if signal == 0 {
        format!("exit {}", (raw >> 8) & 0xff)
    } else {
        format!("signal {signal}")
    }
}

impl AgentChild {
    pub(crate) fn spawn(
        executable: &Path,
        arguments: &[&str],
        directory: Option<&Path>,
        environment: &BTreeMap<String, String>,
    ) -> Result<Arc<AgentChild>, AgentRemoteError> {
        let mut command = Command::new(executable);
        command
            .args(arguments)
            .env_clear()
            .envs(environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.process_group(0);
        if let Some(directory) = directory {
            command.current_dir(directory);
        }
        let mut child = command.spawn().map_err(|error| AgentRemoteError::Spawn {
            path: executable.display().to_string(),
            reason: error.to_string(),
        })?;
        let pid = child.id() as i32;
        let stdin: OwnedFd = child.stdin.take().expect("piped stdin").into();
        let stdout: OwnedFd = child.stdout.take().expect("piped stdout").into();
        let stderr: OwnedFd = child.stderr.take().expect("piped stderr").into();
        drop(child);

        let tail = Arc::new(Mutex::new(Vec::new()));
        let kept = tail.clone();
        std::thread::Builder::new()
            .name("granite.agent.stderr".into())
            .spawn(move || {
                let mut stderr = File::from(stderr);
                let mut chunk = [0u8; 4096];
                while let Ok(count) = stderr.read(&mut chunk) {
                    if count == 0 {
                        break;
                    }
                    let mut tail = kept.lock().unwrap();
                    tail.extend_from_slice(&chunk[..count]);
                    if tail.len() > 8192 {
                        let cut = tail.len() - 8192;
                        tail.drain(..cut);
                    }
                }
            })
            .expect("granite: the stderr reader could not start");

        let (tx, ended) = watch::channel(None);
        std::thread::Builder::new()
            .name("granite.agent.reaper".into())
            .spawn(move || {
                let mut status = 0;
                // SAFETY: rusage is plain data; wait4 fills it for our own child.
                let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
                loop {
                    let reaped = unsafe { libc::wait4(pid, &mut status, 0, &mut usage) };
                    if reaped == -1
                        && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                    {
                        continue;
                    }
                    break;
                }
                let seconds = |time: libc::timeval| time.tv_sec as f64 + time.tv_usec as f64 / 1e6;
                // ru_maxrss is bytes on macOS and kilobytes on Linux.
                let peak = if cfg!(target_os = "linux") {
                    usage.ru_maxrss as i64 * 1024
                } else {
                    usage.ru_maxrss as i64
                };
                let _ = tx.send(Some(Ended {
                    status,
                    cpu_seconds: seconds(usage.ru_utime) + seconds(usage.ru_stime),
                    peak_bytes: peak,
                }));
            })
            .expect("granite: the reaper could not start");

        let channel = PipeChannel::new(stdout, stdin, |line| {
            eprintln!("granite-agent: an unreadable line from the runner: {line}")
        });
        Ok(Arc::new(AgentChild {
            pid,
            channel,
            started: Instant::now(),
            stderr: tail,
            ended,
        }))
    }

    /// Waits for the process to end; at once when it has.
    pub(crate) async fn exited(&self) -> Ended {
        let mut ended = self.ended.clone();
        loop {
            if let Some(done) = *ended.borrow_and_update() {
                return done;
            }
            if ended.changed().await.is_err() {
                return Ended {
                    status: 0,
                    cpu_seconds: 0.0,
                    peak_bytes: 0,
                };
            }
        }
    }

    pub(crate) fn has_exited(&self) -> bool {
        self.ended.borrow().is_some()
    }

    /// SIGTERM to the process's group, then SIGKILL after `grace` if it is still there.
    pub(crate) fn terminate(self: &Arc<Self>, grace: Duration) {
        if self.has_exited() {
            return;
        }
        // SAFETY: signalling our own child's process group.
        unsafe { libc::kill(-self.pid, libc::SIGTERM) };
        let child = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(grace);
            if !child.has_exited() {
                unsafe { libc::kill(-child.pid, libc::SIGKILL) };
            }
        });
    }

    /// The end of what the process wrote to stderr, at most six lines.
    pub(crate) fn log(&self) -> String {
        let tail = String::from_utf8_lossy(&self.stderr.lock().unwrap()).into_owned();
        let lines: Vec<&str> = tail.lines().collect();
        lines[lines.len().saturating_sub(6)..]
            .join("\n")
            .trim()
            .to_owned()
    }

    pub(crate) fn cost(&self, ended: &Ended, asks: u32) -> AgentProcessCost {
        AgentProcessCost {
            seconds: self.started.elapsed().as_secs_f64(),
            cpu_seconds: ended.cpu_seconds,
            peak_bytes: ended.peak_bytes,
            status: describe(ended.status),
            asks,
        }
    }
}
