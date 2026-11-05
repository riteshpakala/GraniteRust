//! What tests of Granite code share.

use std::time::{Duration, Instant};

/// Waits until `condition` holds, polling every 5 ms, for at most `timeout`. Says whether it held.
pub async fn until(condition: impl Fn() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
