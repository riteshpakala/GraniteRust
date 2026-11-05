//! The single writer's thread: what `MainActor` and `DispatchQueue.main` are to Swift Granite.
//!
//! PIN: A TURN NEVER AWAITS. Every commit, drain and settle is a synchronous turn on one thread, in the
//! order it was sent, so a reducer's write and an agent's drain never interleave. Work that awaits is
//! spawned on [`Main::runtime`] and sends back.
//! PIN: MAIN OWNS ITS RUNTIME. Effects and runs are spawned on a runtime the main thread keeps alive, so
//! nothing depends on whichever runtime the caller happens to be in (a test's, a host's).

use std::future::Future;
use std::sync::{Arc, OnceLock, mpsc};
use std::thread::{self, ThreadId};

use tokio::runtime::Handle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

type Turn = Box<dyn FnOnce() + Send + 'static>;

/// The main thread and the runtime its effects run on. Clones share one thread.
#[derive(Clone)]
pub struct Main {
    inner: Arc<Inner>,
}

struct Inner {
    tx: mpsc::Sender<Turn>,
    thread: ThreadId,
    handle: Handle,
}

impl Main {
    /// A main thread of its own (`granite.main`) and a runtime for its work (`granite.work`). The thread
    /// ends once every handle is gone.
    pub fn spawn() -> Main {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("granite.work")
            .enable_all()
            .build()
            .expect("granite: the runtime for main's work could not start");
        let handle = runtime.handle().clone();
        let (tx, rx) = mpsc::channel::<Turn>();
        let (started, thread) = mpsc::channel();
        thread::Builder::new()
            .name("granite.main".into())
            .spawn(move || {
                let _ = started.send(thread::current().id());
                let entered = runtime.enter();
                for turn in rx {
                    turn();
                }
                drop(entered);
                runtime.shutdown_background();
            })
            .expect("granite: the main thread could not start");
        let thread = thread
            .recv()
            .expect("granite: the main thread did not start");
        Main {
            inner: Arc::new(Inner { tx, thread, handle }),
        }
    }

    /// The process's main, made on first use: what a relay, a package host and a runner use.
    pub fn global() -> &'static Main {
        static GLOBAL: OnceLock<Main> = OnceLock::new();
        GLOBAL.get_or_init(Main::spawn)
    }

    /// Whether the calling thread is this main.
    pub fn is_main(&self) -> bool {
        thread::current().id() == self.inner.thread
    }

    /// Runs `turn` on main after everything sent before it. Always deferred, even from main.
    pub fn enqueue(&self, turn: impl FnOnce() + Send + 'static) {
        let _ = self.inner.tx.send(Box::new(turn));
    }

    /// Runs `turn` now when the caller is on main, else on main's next turn (Swift's `onMain`).
    pub fn run(&self, turn: impl FnOnce() + Send + 'static) {
        if self.is_main() {
            turn()
        } else {
            self.enqueue(turn)
        }
    }

    /// `f`'s value, computed on main after everything sent before it.
    pub async fn call<T: Send + 'static>(&self, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = oneshot::channel();
        self.enqueue(move || {
            let _ = tx.send(f());
        });
        rx.await.expect("granite: the main thread ended")
    }

    /// Returns once main has run every turn sent to it before the call.
    pub async fn turn(&self) {
        self.call(|| ()).await
    }

    /// The runtime effects and runs are spawned on.
    pub fn runtime(&self) -> &Handle {
        &self.inner.handle
    }

    /// Spawns `future` on main's runtime.
    pub fn spawn_work<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.inner.handle.spawn(future)
    }

    /// Runs `future` to its end on main's runtime, from a thread outside any runtime (a test, a `main`).
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.inner.handle.block_on(future)
    }
}
