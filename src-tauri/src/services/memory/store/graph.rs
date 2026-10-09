//! Relations domain: entity-relation write and query between facts.
use super::*;
use crate::services::memory::CollectRowsWarn;

impl MemoryStore {
    /// Overwrites the relation triples of a fact (delete-then-insert, idempotent).
    pub fn set_relations(
        &self,
        fact_id: &str,
        relations: &[(String, String, String)],
    ) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM facts_relations WHERE fact_id = ?1", [fact_id])
            .map_err(|e| e.to_string())?;
        let now = now_iso();
        for (e1, rel, e2) in relations {
            if e1.trim().is_empty() || e2.trim().is_empty() {
                continue;
            }
            tx.execute(
                "INSERT INTO facts_relations (fact_id, entity1, relation, entity2, valid_from, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![fact_id, e1.trim(), rel.trim(), e2.trim(), now, now],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    /// Reads all relation triples of a fact.
    pub fn relations_for(&self, fact_id: &str) -> Result<Vec<(String, String, String)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT entity1, relation, entity2 FROM facts_relations WHERE fact_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([fact_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        Ok(rows.collect_rows("graph"))
    }

    /// Counts mutual containment between query terms and entity text (lightweight entity hits; used for retrieval score boosting).
    ///
    /// Returns (fact_id -> matched relation count). Match rule: query contains entity or entity contains query (entity length >= 2).
    pub fn relation_entity_hits(
        &self,
        query: &str,
    ) -> Result<std::collections::HashMap<String, usize>, String> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT fact_id, entity1, entity2 FROM facts_relations")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut hits: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for row in rows.collect_rows("graph") {
            let matched = [row.1.as_str(), row.2.as_str()]
                .iter()
                .filter(|e| e.len() >= 2 && (q.contains(**e) || e.contains(q)))
                .count();
            if matched > 0 {
                *hits.entry(row.0).or_insert(0) += matched;
            }
        }
        Ok(hits)
    }
}
