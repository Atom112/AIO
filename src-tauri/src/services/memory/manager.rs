//! Per-project memory store connection management: an LRU cache, avoiding frequent open/close churn and file-lock contention.

use crate::services::memory::store::MemoryStore;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Instant;

const MAX_STORES: usize = 32;

/// MemoryStoreManager: an LRU cache mapping project_root to Arc<MemoryStore>.
pub struct MemoryStoreManager {
    stores: DashMap<String, Arc<MemoryStore>>,
    touched: DashMap<String, Instant>,
}

impl Default for MemoryStoreManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStoreManager {
    /// Create a new manager.
    pub fn new() -> Self {
        Self {
            stores: DashMap::new(),
            touched: DashMap::new(),
        }
    }

    /// Open (or reuse) the memory store for the given project; project_root is the project's absolute path.
    pub fn open(&self, project_root: &str) -> Result<Arc<MemoryStore>, String> {
        let key = project_root.to_string();
        // MEM-07: `entry()` is atomic per-key — the old get-then-insert raced on a
        // concurrent miss: both threads saw "absent", both opened a SQLite
        // connection, and the loser's Arc was silently dropped while its callers
        // kept using the orphaned connection.
        let store: Arc<MemoryStore> = match self.stores.entry(key.clone()) {
            dashmap::mapref::entry::Entry::Occupied(o) => o.get().clone(),
            dashmap::mapref::entry::Entry::Vacant(v) => {
                let s = Arc::new(MemoryStore::open(project_root)?);
                v.insert(s.clone());
                s
            }
        };
        self.touched.insert(key, Instant::now());
        self.evict_if_needed();
        Ok(store)
    }

    /// Remove the cached connection when a project is deleted (called when the project directory is removed).
    pub fn remove(&self, project_root: &str) {
        self.stores.remove(project_root);
        self.touched.remove(project_root);
    }

    /// Cache size governance (MEM-07: real LRU). Once the cap is exceeded the
    /// least-recently-touched entries are evicted — TTL alone never bound the cache:
    /// with everything touched recently, nothing expired and the cache grew forever.
    fn evict_if_needed(&self) {
        if self.stores.len() <= MAX_STORES {
            return;
        }
        self.evict_over_cap(MAX_STORES);
    }

    /// True-LRU eviction down to `cap`: evicts the oldest-touched entries regardless
    /// of TTL (`cap` is a parameter for testability; production uses [`MAX_STORES`]).
    /// Dropping the cached Arc closes the SQLite connection only after in-flight
    /// users finish with it (graceful).
    pub(crate) fn evict_over_cap(&self, cap: usize) {
        if self.stores.len() <= cap {
            return;
        }
        let to_remove = self.stores.len() - cap;
        let mut order: Vec<(String, Instant)> = self
            .touched
            .iter()
            .map(|e| (e.key().clone(), *e.value()))
            .collect();
        order.sort_by_key(|(_, t)| *t);
        order.into_iter().take(to_remove).for_each(|(key, _)| {
            self.stores.remove(&key);
            self.touched.remove(&key);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MEM-7: concurrent-miss hardening — `entry()` makes the open path atomic
    /// per key; a repeated open must always return the SAME Arc (the old
    /// get-then-insert could create two live SQLite connections on a racing miss).
    #[test]
    fn repeated_open_returns_the_same_store() {
        let mgr = MemoryStoreManager::new();
        let dir = std::env::temp_dir().join(format!("aio-memmgr-same-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = mgr.open(&dir.to_string_lossy()).unwrap();
        let b = mgr.open(&dir.to_string_lossy()).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "同一项目必须复用同一连接实例");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// MEM-07: the cache must be bounded even when every entry was touched recently
    /// (the old TTL-only policy let an all-hot cache grow forever). True LRU: the
    /// least-recently-touched entry is evicted once the cap is exceeded.
    #[test]
    fn lru_eviction_is_real_even_when_all_hot() {
        let mgr = MemoryStoreManager::new();
        let base = std::env::temp_dir().join(format!("aio-memmgr-{}", std::process::id()));
        let mut paths = Vec::new();
        for i in 0..3 {
            let dir = base.join(format!("p{i}"));
            std::fs::create_dir_all(&dir).unwrap();
            mgr.open(&dir.to_string_lossy()).unwrap();
            paths.push(dir);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(mgr.stores.len(), 3);
        mgr.evict_over_cap(2);
        assert_eq!(mgr.stores.len(), 2, "超上限必须真实淘汰");
        let key_of = |p: &std::path::Path| p.to_string_lossy().into_owned();
        assert!(
            mgr.stores.get(&key_of(&paths[0])).is_none(),
            "最久未触的条目应被淘汰"
        );
        assert!(
            mgr.stores.get(&key_of(&paths[2])).is_some(),
            "最近触达的条目应保留"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
