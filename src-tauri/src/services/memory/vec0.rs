//! sqlite-vec (vec0 virtual table) integration: in-SQLite vector search acceleration.
//!
//! Registers vec0 on all new connections via sqlite3_auto_extension; when unavailable
//! (build/platform limits) it falls back to vector.rs brute-force cosine (equivalent behavior, slower).

use rusqlite::Connection;

use super::vector::f32_to_bytes;
use crate::ierr;
use crate::services::memory::CollectRowsWarn;

static VEC0_ONCE: std::sync::Once = std::sync::Once::new();
static VEC0_OK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Try to register the sqlite-vec extension (once per process); returns whether it is available.
/// Must be called before any SQLite connection is created, or the auto extension will not reach existing connections.
#[allow(clippy::missing_transmute_annotations)] // usage matching the sqlite-vec crate's own tests
pub fn vec0_available() -> bool {
    VEC0_ONCE.call_once(|| {
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            let ok = unsafe {
                // sqlite-vec's init is a 3-arg C entry point (loadable extension spec);
                // auto_extension needs a 1-arg entry, hence the transmute, matching the crate's own tests.
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

/// Create the vec0 table for a given dimension (drop and recreate on dimension change; re-indexing old vectors is up to the caller).
pub fn ensure_vec_table(conn: &Connection, dim: usize) -> Result<(), String> {
    let dim = dim.clamp(1, 8192);
    let sql = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_facts USING vec0(embedding float[{dim}] distance_metric=cosine, fact_id text +)"
    );
    conn.execute_batch(&sql)
        .map_err(|e| ierr!("创建 vec0 表失败: {}", "Failed to create vec0 table: {}", e))
}

/// vec0 kNN search: returns (fact_id, cosine_similarity) sorted by similarity descending.
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
    Ok(rows.collect_rows("vec0"))
}

/// vec0 upsert (delete-then-insert by fact_id).
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

/// vec0 delete by fact_id.
pub fn vec0_delete(conn: &Connection, fact_id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM vec_facts WHERE fact_id = ?1", [fact_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Clear the vec0 table (called before re-indexing).
pub fn vec0_clear(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM vec_facts", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Read the current dimension of the vec0 table (returns None if it does not exist).
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

/// vec_code upsert (delete-then-insert by chunk_id).
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

/// vec_code kNN search: returns (chunk_id, cosine_similarity).
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
    Ok(rows.collect_rows("vec0"))
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
