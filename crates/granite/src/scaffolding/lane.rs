//! A serial async queue: Swift's `GraniteLane`. Bodies run one after another, in arrival order.
//!
//! PIN: Dropping a caller's future drops its body (or its place in line); the rest of the queue goes on.

use std::future::Future;

pub struct Lane(tokio::sync::Mutex<()>);

impl Default for Lane {
    fn default() -> Self {
        Self::new()
    }
}

impl Lane {
    pub fn new() -> Lane {
        Lane(tokio::sync::Mutex::new(()))
    }

    pub async fn run<T>(&self, body: impl Future<Output = T>) -> T {
        let _turn = self.0.lock().await;
        body.await
    }
}
