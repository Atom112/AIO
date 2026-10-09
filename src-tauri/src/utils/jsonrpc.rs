//! JSON-RPC pending-request map — shared mechanism for the MCP and LSP clients.
//!
//! Provides atomic id allocation plus a `DashMap<u64, oneshot::Sender<T>>` with
//! registration / response dispatch / timeout-and-cancel cleanup.

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::oneshot;

/// Reason a response wait failed (callers map it to their own error types).
#[derive(Debug, Clone, Copy)]
pub enum WaitError {
    /// Sender dropped (connection closed / response channel broken)
    Canceled,
    /// Timed out (entry already cleaned up)
    Timeout(Duration),
}

/// Pending map from id to oneshot sender (with atomic id allocation).
pub struct PendingMap<T> {
    map: DashMap<u64, oneshot::Sender<T>>,
    next_id: AtomicU64,
}

impl<T> Default for PendingMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PendingMap<T> {
    pub fn new() -> Self {
        Self {
            map: DashMap::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Allocate an id and register a wait slot; returns (id, receiver).
    pub fn register(&self) -> (u64, oneshot::Receiver<T>) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.map.insert(id, tx);
        (id, rx)
    }

    /// Take the sender when a response arrives (one response per request; first come, first served — can be taken only once).
    pub fn take(&self, id: u64) -> Option<oneshot::Sender<T>> {
        self.map.remove(&id).map(|(_, tx)| tx)
    }

    /// Cancel a registration (cleaned up on send failure / timeout / drop to avoid leaks).
    pub fn cancel(&self, id: u64) {
        self.map.remove(&id);
    }

    /// Wait for a response; on timeout or sender drop, cleans up the entry and returns [`WaitError`].
    pub async fn wait(
        &self,
        id: u64,
        rx: oneshot::Receiver<T>,
        dur: Duration,
    ) -> Result<T, WaitError> {
        match tokio::time::timeout(dur, rx).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => {
                self.cancel(id);
                Err(WaitError::Canceled)
            }
            Err(_) => {
                self.cancel(id);
                Err(WaitError::Timeout(dur))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Duration as D;

    #[tokio::test]
    async fn register_take_and_wait_ok() {
        let map = PendingMap::<u32>::new();
        let (id1, rx1) = map.register();
        let (id2, rx2) = map.register();
        assert_eq!(id1 + 1, id2);
        let tx = map.take(id1).expect("应有待发端");
        tx.send(42).unwrap();
        assert_eq!(map.wait(id1, rx1, D::from_secs(1)).await.unwrap(), 42);
        assert!(map.take(id1).is_none(), "发送端只能取一次");
        assert!(map.take(id2).is_some(), "其余表项不受影响");
        drop(rx2);
    }

    #[tokio::test]
    async fn timeout_cancels_the_entry() {
        let map = PendingMap::<u32>::new();
        let (id, rx) = map.register();
        let err = map.wait(id, rx, D::from_millis(10)).await.unwrap_err();
        assert!(matches!(err, WaitError::Timeout(_)));
        assert!(map.take(id).is_none(), "超时后表项必须被清理");
    }

    #[tokio::test]
    async fn cancel_removes_entry() {
        let map = PendingMap::<u32>::new();
        let (id, _rx) = map.register();
        map.cancel(id);
        assert!(map.take(id).is_none());
    }
}
