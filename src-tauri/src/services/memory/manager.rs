//! Per-project memory store connection management: an LRU cache, avoiding frequent open/close churn and file-lock contention.

use crate::services::memory::store::MemoryStore;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_STORES: usize = 32;
const IDLE_TTL: Duration = Duration::from_secs(30 * 60);

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
        if let Some(store) = self.stores.get(&key) {
            self.touched.insert(key, Instant::now());
            return Ok(store.clone());
        }
        let store = Arc::new(MemoryStore::open(project_root)?);
        self.stores.insert(key.clone(), store.clone());
        self.touched.insert(key, Instant::now());
        self.evict_if_needed();
        Ok(store)
    }

    /// Remove the cached connection when a project is deleted (called when the project directory is removed).
    pub fn remove(&self, project_root: &str) {
        self.stores.remove(project_root);
        self.touched.remove(project_root);
    }

    /// Cache size governance: evict entries idle beyond the TTL once the cap is exceeded.
    fn evict_if_needed(&self) {
        if self.stores.len() <= MAX_STORES {
            return;
        }
        let now = Instant::now();
        let mut candidates: Vec<(String, Instant)> = Vec::new();
        for entry in self.touched.iter() {
            if now.duration_since(*entry.value()) > IDLE_TTL {
                candidates.push((entry.key().clone(), *entry.value()));
            }
        }
        candidates.sort_by_key(|a| a.1);
        let mut to_remove = self.stores.len().saturating_sub(MAX_STORES);
        for (key, _) in candidates {
            if to_remove == 0 {
                break;
            }
            self.stores.remove(&key);
            self.touched.remove(&key);
            to_remove -= 1;
        }
    }
}
