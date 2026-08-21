//! sqlite-vec（vec0 虚拟表）接入：P3 的 SQLite 内向量检索加速。
//!
//! 通过 sqlite3_auto_extension 把 vec0 注册到所有新建连接；不可用（编译/平台限制）时
//! 自动回退到 vector.rs 的暴力余弦，功能等价、仅性能差异。

use rusqlite::Connection;

use super::vector::f32_to_bytes;

static VEC0_ONCE: std::sync::Once = std::sync::Once::new();
static VEC0_OK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 尝试注册 sqlite-vec 扩展（进程级一次），返回是否可用。
/// 必须在创建任何 SQLite 连接之前调用，否则自动扩展不会注册到已存在的连接。
#[allow(clippy::missing_transmute_annotations)] // 与 sqlite-vec crate 自带测试一致的用法
pub fn vec0_available() -> bool {
    VEC0_ONCE.call_once(|| {
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            let ok = unsafe {
                // sqlite-vec 的 init 是 3 参 C 入口（loadable 规范）；
                // auto_extension 需要 1 参入口，与 crate 自带测试一致的 transmute 用法。
                rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                    sqlite_vec::sqlite3_vec_init as *const (),
                ))) == rusqlite::ffi::SQLITE_OK
            };
            VEC0_OK.store(ok, std::sync::atomic::Ordering::Relaxed);
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        VEC0_OK.store(false, std::sync::atomic::Ordering::Relaxed);
    });
    VEC0_OK.load(std::sync::atomic::Ordering::Relaxed)
}

/// 按维度创建 vec0 表（维度变化时先删后建；旧向量由调用方决定是否重索引）。
pub fn ensure_vec_table(conn: &Connection, dim: usize) -> Result<(), String> {
    let dim = dim.clamp(1, 8192);
    let sql = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_facts USING vec0(embedding float[{dim}] distance_metric=cosine, fact_id text +)"
    );
    conn.execute_batch(&sql)
        .map_err(|e| format!("创建 vec0 表失败: {e}"))
}

/// vec0 kNN 检索：返回 (fact_id, cosine_similarity) 按相似度降序。
pub fn vec0_search(
    conn: &Connection,
    query: &[f32],
    k: usize,
) -> Result<Vec<(String, f32)>, String> {
    let sql = "SELECT fact_id, 1.0 - distance FROM vec_facts WHERE embedding MATCH ?1 AND k = ?2";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![f32_to_bytes(query), k as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f32>(1)?))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// vec0 写入（按 fact_id 先删后插）。
pub fn vec0_upsert(conn: &Connection, fact_id: &str, vec: &[f32]) -> Result<(), String> {
    conn.execute("DELETE FROM vec_facts WHERE fact_id = ?1", [fact_id])
        .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO vec_facts (embedding, fact_id) VALUES (?1, ?2)",
        rusqlite::params![f32_to_bytes(vec), fact_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// vec0 删除（按 fact_id）。
pub fn vec0_delete(conn: &Connection, fact_id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM vec_facts WHERE fact_id = ?1", [fact_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 清空 vec0 表（重索引前调用）。
pub fn vec0_clear(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM vec_facts", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 读取 vec0 表当前维度（不存在时返回 None）。
pub fn vec0_dim(conn: &Connection) -> Option<usize> {
    let sql = "SELECT sql FROM sqlite_master WHERE type = ?1 AND name = ?2";
    let row: Option<String> = conn
        .query_row(sql, rusqlite::params!["table", "vec_facts"], |r| r.get(0))
        .ok();
    let sql_text = row?;
    let start = sql_text.find("float[")?;
    let rest = &sql_text[start + 6..];
    let end = rest.find(']')?;
    rest[..end].parse::<usize>().ok()
}

/// vec_code 写入（按 chunk_id 先删后插）。
pub fn vec0_upsert_code(conn: &Connection, chunk_id: &str, vec: &[f32]) -> Result<(), String> {
    conn.execute("DELETE FROM vec_code WHERE chunk_id = ?1", [chunk_id])
        .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO vec_code (embedding, chunk_id) VALUES (?1, ?2)",
        rusqlite::params![f32_to_bytes(vec), chunk_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// vec_code kNN 检索：返回 (chunk_id, cosine_similarity)。
pub fn vec0_search_code(
    conn: &Connection,
    query: &[f32],
    k: usize,
) -> Result<Vec<(String, f32)>, String> {
    let sql = "SELECT chunk_id, 1.0 - distance FROM vec_code WHERE embedding MATCH ?1 AND k = ?2";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![f32_to_bytes(query), k as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f32>(1)?))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_vec0() {
        if !vec0_available() {
            return;
        }
        let conn = Connection::open_in_memory().unwrap();
        ensure_vec_table(&conn, 4).unwrap();
        vec0_upsert(&conn, "a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        vec0_upsert(&conn, "b", &[0.0, 1.0, 0.0, 0.0]).unwrap();
        let res = vec0_search(&conn, &[1.0, 0.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].0, "a");
        assert!((res[0].1 - 1.0).abs() < 1e-5);
        vec0_delete(&conn, "a").unwrap();
        let res2 = vec0_search(&conn, &[1.0, 0.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(res2.len(), 1);
        assert_eq!(res2[0].0, "b");
    }
}
