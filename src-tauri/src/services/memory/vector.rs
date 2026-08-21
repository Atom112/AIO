//! 向量编解码与余弦相似度（P1 用 Rust 暴力打分，P3 可替换为 sqlite-vec）。

/// 将 f32 向量编码为小端 float32 字节序列（SQLite BLOB 存储格式）。
pub fn f32_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// 将 SQLite BLOB 解码为 f32 向量；尾部不足 4 字节的残段忽略。
pub fn bytes_to_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// 余弦相似度：维度不一致或为空时返回 0（不可比）。
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += (*x as f64) * (*y as f64);
        na += (*x as f64) * (*x as f64);
        nb += (*y as f64) * (*y as f64);
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())) as f32
}

/// 暴力余弦 top-k：对 (id, 向量) 列表打分，返回按分数降序的前 k 个 (id, score)。
pub fn brute_force_topk(
    rows: &[(String, Vec<f32>)],
    query: &[f32],
    k: usize,
) -> Vec<(String, f32)> {
    let mut scored: Vec<(String, f32)> = rows
        .iter()
        .map(|(id, v)| (id.clone(), cosine(query, v)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let v = vec![0.5f32, -1.25, 3.0, 0.0];
        let bytes = f32_to_bytes(&v);
        assert_eq!(bytes.len(), 16);
        assert_eq!(bytes_to_f32(&bytes), v);
    }

    #[test]
    fn test_cosine() {
        let a = vec![1.0f32, 0.0];
        let b = vec![1.0f32, 0.0];
        assert!((cosine(&a, &b) - 1.0).abs() < 1e-5);
        let c = vec![0.0f32, 1.0];
        assert!(cosine(&a, &c).abs() < 1e-5);
        assert_eq!(cosine(&[1.0f32], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine(&Vec::<f32>::new(), &Vec::<f32>::new()), 0.0);
    }

    #[test]
    fn test_topk_order() {
        let rows = vec![
            ("a".to_string(), vec![1.0f32, 0.0]),
            ("b".to_string(), vec![0.0f32, 1.0]),
            ("c".to_string(), vec![0.9f32, 0.1]),
        ];
        let top = brute_force_topk(&rows, &[1.0f32, 0.0], 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "a");
        assert_eq!(top[1].0, "c");
    }
}
