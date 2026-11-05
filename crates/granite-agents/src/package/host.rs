//! A package's one center in a process: Swift Granite's `AgentPackageHost`. Made on first use on
//! [`granite::Main::global`], with the package's compute made once, on its first call.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use granite::{Agent, Center, Main};
use tokio::sync::OnceCell;

use super::{AgentPackage, PackageRun, PackageState};

pub struct PackageHost<P: AgentPackage> {
    pub center: Center<PackageState>,
    pub agent: Agent<PackageRun<P>>,
}

impl<P: AgentPackage> PackageHost<P> {
    /// The process's host for `P`.
    pub fn shared() -> Arc<PackageHost<P>> {
        static HOSTS: OnceLock<Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>> =
            OnceLock::new();
        let hosts = HOSTS.get_or_init(Default::default);
        let mut hosts = hosts.lock().unwrap();
        if let Some(found) = hosts.get(&TypeId::of::<P>()) {
            return found
                .clone()
                .downcast::<PackageHost<P>>()
                .expect("granite: a package host of another type");
        }
        let center = Center::<PackageState>::new(Main::global()).labelled(P::WORD);
        let agent = Agent::<PackageRun<P>>::new(&center);
        let compute: Arc<OnceCell<P::Compute>> = Arc::new(OnceCell::new());
        agent.attach(move |_| {
            let compute = compute.clone();
            async move { compute.get_or_try_init(P::compute).await.cloned() }
        });
        let host = Arc::new(PackageHost { center, agent });
        hosts.insert(TypeId::of::<P>(), host.clone());
        host
    }
}
