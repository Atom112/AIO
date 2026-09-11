//! JSON-RPC 待响应请求表（pending map）—— MCP 与 LSP 客户端公共机制。
//!
//! 两边此前各自实现同一套：id 原子计数 + `DashMap<u64, oneshot::Sender<T>>`
//! 注册/响应分发/超时取消清理（MCP connection.rs 与 LSP client.rs 各一份，易漂移）。

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::oneshot;

/// 等待响应失败的原因（调用方映射到自己的错误类型）。
#[derive(Debug, Clone, Copy)]
pub enum WaitError {
    /// 发送端被丢弃（连接关闭 / 响应通道断开）
    Canceled,
    /// 超时（表项已自动清理）
    Timeout(Duration),
}

/// id → oneshot 发送端的待响应表（含 id 原子分配）。
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

    /// 分配一个 id 并注册等待槽位，返回 (id, 接收端)。
    pub fn register(&self) -> (u64, oneshot::Receiver<T>) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.map.insert(id, tx);
        (id, rx)
    }

    /// 响应到达时取出发送端（notion：响应与请求一一对应，先到先得只能取一次）。
    pub fn take(&self, id: u64) -> Option<oneshot::Sender<T>> {
        self.map.remove(&id).map(|(_, tx)| tx)
    }

    /// 取消注册（发送失败 / 超时 / 被丢弃时清理，避免泄漏）。
    pub fn cancel(&self, id: u64) {
        self.map.remove(&id);
    }

    /// 等待响应；超时或发送端被丢弃时自动清理表项后返回 [`WaitError`]。
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
