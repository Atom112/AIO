//! Semantic embedding cache: model + text hash -> vector, capacity cap + FIFO eviction.
//!
//! Shared by the write and lookup sides: identical text (fact content / query) need not repeat online API calls or local inference across sessions.
//! Keys include the model name and target dims (vectors with different post-Matryoshka truncation dims are never mixed). Bulk reindex paths bypass the cache.

use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const CACHE_CAP: usize = 2048;

struct EmbedCache {
    map: DashMap<String, Vec<f32>>,
    /// FIFO eviction order (insertion order)
    order: Mutex<VecDeque<String>>,
}

fn cache() -> &'static EmbedCache {
    static CACHE: OnceLock<EmbedCache> = OnceLock::new();
    CACHE.get_or_init(|| EmbedCache {
        map: DashMap::new(),
        order: Mutex::new(VecDeque::new()),
    })
}

fn key(model: &str, dims: usize, text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    hasher.update(b"\0");
    hasher.update(dims.to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(text.as_bytes());
    format!(
        "{}|{}:{}",
        model,
        dims,
        hex::encode(&hasher.finalize()[..8])
    )
}

/// Cache lookup (vectors are bound to model/dims; a dim mismatch counts as a miss).
pub fn get(model: &str, dims: usize, text: &str) -> Option<Vec<f32>> {
    cache().map.get(&key(model, dims, text)).map(|v| v.clone())
}

/// Cache write (capacity cap; evicts the oldest entry when exceeded).
pub fn put(model: &str, dims: usize, text: &str, v: Vec<f32>) {
    let c = cache();
    let k = key(model, dims, text);
    if c.map.contains_key(&k) {
        return;
    }
    c.map.insert(k.clone(), v);
    let mut order = match c.order.lock() {
        Ok(o) => o,
        Err(_) => return,
    };
    order.push_back(k.clone());
    while order.len() > CACHE_CAP {
        if let Some(oldest) = order.pop_front() {
            c.map.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_roundtrip_and_dim_isolation() {
        put("m1", 384, "你好", vec![1.0, 2.0, 3.0]);
        assert_eq!(get("m1", 384, "你好"), Some(vec![1.0, 2.0, 3.0]));
        // Different dims/models do not share entries
        assert_eq!(get("m1", 512, "你好"), None);
        assert_eq!(get("m2", 384, "你好"), None);
        assert_eq!(get("m1", 384, "其他"), None);
    }
}
