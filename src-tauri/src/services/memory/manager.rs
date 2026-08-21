//! 每项目记忆库连接管理：LRU 缓存，避免高频开关连接与文件锁竞争。

use crate::services::memory::store::MemoryStore;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_STORES: usize = 32;
const IDLE_TTL: Duration = Duration::from_secs(30 * 60);

/// MemoryStoreManager：project_root → Arc<MemoryStore> 的 LRU 缓存。
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
    /// 新建管理器。
    pub fn new() -> Self {
        Self {
            stores: DashMap::new(),
            touched: DashMap::new(),
        }
    }

    /// 打开（或复用）指定项目的记忆库；project_root 为项目绝对路径。
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

    /// 项目删除时移除缓存连接（删除项目目录时调用）。
    pub fn remove(&self, project_root: &str) {
        self.stores.remove(project_root);
        self.touched.remove(project_root);
    }

    /// 缓存规模治理：超过上限时驱逐超时未访问的项。
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
