//! 语义嵌入缓存：模型 + 文本哈希 → 向量，容量上限 + FIFO 淘汰。
//!
//! 写入与查询两侧共用：同一文本（事实内容/查询）不必跨会话重复调用在线 API 或本地推理。
//! 键包含模型名与目标维度（Matryoshka 裁剪后维度不同的向量不混用）。批量重索引路径不走缓存。

use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const CACHE_CAP: usize = 2048;

struct EmbedCache {
    map: DashMap<String, Vec<f32>>,
    /// FIFO 淘汰序（插入顺序）
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

/// 命中缓存（向量与模型/维度绑定，维度不一致视为未命中）。
pub fn get(model: &str, dims: usize, text: &str) -> Option<Vec<f32>> {
    cache().map.get(&key(model, dims, text)).map(|v| v.clone())
}

/// 写入缓存（容量上限，超出淘汰最旧条目）。
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
        // 不同维度/不同模型不共享
        assert_eq!(get("m1", 512, "你好"), None);
        assert_eq!(get("m2", 384, "你好"), None);
        assert_eq!(get("m1", 384, "其他"), None);
    }
}
