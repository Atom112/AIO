//! RAG retrieval quality benchmark (optional in CI):
//!
//! ```text
//! cargo test --lib eval_rag -- --ignored --nocapture
//! ```
//!
//! Reads `tests/rag_eval/dataset.json` (40 facts + 12 labeled queries, mixed zh/EN),
//! builds a temporary memory store with the built-in local embedding model, and compares three retrieval routes:
//!   vector-only / FTS (sparse leg) / hybrid (RRF + recency)
//! Prints an NDCG@10 / MRR / P@5 comparison table (a fourth "hybrid+rerank" route is evaluated separately once the local rerank model has been downloaded).
//! Results are informational only: no hard assertions, avoiding brittle CI failures.

use crate::plugins::embed::Embedder;
use crate::services::memory::store::{MemoryStore, SearchOptions};
use serde::Deserialize;
use std::collections::HashMap;

const DATASET: &str = include_str!("../../../tests/rag_eval/dataset.json");

#[derive(Deserialize)]
struct Dataset {
    facts: Vec<Fact>,
    queries: Vec<Query>,
}

#[derive(Deserialize)]
struct Fact {
    label: String,
    category: String,
    content: String,
}

#[derive(Deserialize)]
struct Query {
    query: String,
    relevant: Vec<String>,
}

fn ndcg_at_k(ranked: &[String], relevant: &[String], k: usize) -> f64 {
    let k = k.min(ranked.len());
    let mut dcg = 0.0;
    for (i, id) in ranked.iter().take(k).enumerate() {
        let rel = if relevant.iter().any(|r| r == id) {
            1.0
        } else {
            0.0
        };
        dcg += rel / (i as f64 + 2.0).log2();
    }
    let mut idcg = 0.0;
    for i in 0..relevant.len().min(k) {
        idcg += 1.0 / (i as f64 + 2.0).log2();
    }
    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

fn mrr(ranked: &[String], relevant: &[String]) -> f64 {
    for (i, id) in ranked.iter().enumerate() {
        if relevant.iter().any(|r| r == id) {
            return 1.0 / (i as f64 + 1.0);
        }
    }
    0.0
}

fn p_at_5(ranked: &[String], relevant: &[String]) -> f64 {
    let top = ranked.iter().take(5);
    let hits = top
        .filter(|id| relevant.iter().any(|r| r.as_str() == id.as_str()))
        .count();
    hits as f64 / 5.0
}

async fn build_store(dataset: &Dataset) -> (MemoryStore, HashMap<String, String>) {
    let root = std::env::temp_dir().join(format!("aio-rag-eval-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = MemoryStore::open(root.to_str().unwrap()).unwrap();
    let embedder = crate::plugins::embed::local_onnx::LocalEmbedder::with_dir(
        std::env::temp_dir(),
        &crate::core::models::MemoryEmbeddingConfig::default(),
    );
    let mut label_to_id: HashMap<String, String> = HashMap::new();
    for f in &dataset.facts {
        let Ok(v) = embedder.embed(std::slice::from_ref(&f.content)).await else {
            continue;
        };
        let Some(v) = v.into_iter().next() else {
            continue;
        };
        let saved = store
            .upsert_fact(
                &crate::services::memory::store::NewFact {
                    key: Some(format!("eval:{}", f.label)),
                    content: f.content.clone(),
                    category: f.category.clone(),
                    importance: 0.5,
                    source_type: Some("eval".into()),
                    source_refs: None,
                    pinned: false,
                },
                Some(("all-MiniLM-L6-v2".into(), v.len(), v)),
            )
            .unwrap();
        label_to_id.insert(f.label.clone(), saved.id);
    }
    (store, label_to_id)
}

#[tokio::test]
#[ignore = "RAG 质量基准：cargo test --lib eval_rag -- --ignored --nocapture"]
async fn eval_rag_quality_benchmark() {
    let dataset: Dataset = serde_json::from_str(DATASET).expect("rag_eval dataset.json 解析失败");
    let (store, label_to_id) = build_store(&dataset).await;
    let embedder = crate::plugins::embed::local_onnx::LocalEmbedder::with_dir(
        std::env::temp_dir(),
        &crate::core::models::MemoryEmbeddingConfig::default(),
    );

    struct Route {
        name: &'static str,
        ndcg: Vec<f64>,
        mrr: Vec<f64>,
        p5: Vec<f64>,
    }

    let mut routes: Vec<Route> = vec![
        Route {
            name: "纯向量 (vector-only)",
            ndcg: vec![],
            mrr: vec![],
            p5: vec![],
        },
        Route {
            name: "FTS (keyword-only)",
            ndcg: vec![],
            mrr: vec![],
            p5: vec![],
        },
        Route {
            name: "混合 (hybrid RRF)",
            ndcg: vec![],
            mrr: vec![],
            p5: vec![],
        },
    ];

    let mut q_no = 0usize;
    for q in &dataset.queries {
        q_no += 1;
        let relevant: Vec<String> = q
            .relevant
            .iter()
            .filter_map(|l| label_to_id.get(l).cloned())
            .collect();
        let qv = embedder
            .embed(std::slice::from_ref(&q.query))
            .await
            .ok()
            .and_then(|v| v.into_iter().next());

        // Route 1: vector-only (cosine top-10)
        let vec_ids: Vec<String> = match &qv {
            Some(v) => store
                .search_similar(v, 10)
                .unwrap_or_default()
                .into_iter()
                .map(|(id, _)| id)
                .collect(),
            None => Vec::new(),
        };
        // Route 2: FTS-only
        let fts_ids: Vec<String> = store
            .search_fts_only(
                &q.query,
                &SearchOptions {
                    category: None,
                    k: 10,
                    min_score: None,
                },
            )
            .unwrap_or_default()
            .into_iter()
            .map(|f| f.id)
            .collect();
        // Route 3: hybrid (vector + FTS, RRF)
        let hybrid_ids: Vec<String> = store
            .search_hybrid(
                &q.query,
                qv.as_deref(),
                &SearchOptions {
                    category: None,
                    k: 10,
                    min_score: None,
                },
            )
            .unwrap_or_default()
            .into_iter()
            .map(|f| f.id)
            .collect();

        routes[0].ndcg.push(ndcg_at_k(&vec_ids, &relevant, 10));
        routes[0].mrr.push(mrr(&vec_ids, &relevant));
        routes[0].p5.push(p_at_5(&vec_ids, &relevant));
        routes[1].ndcg.push(ndcg_at_k(&fts_ids, &relevant, 10));
        routes[1].mrr.push(mrr(&fts_ids, &relevant));
        routes[1].p5.push(p_at_5(&fts_ids, &relevant));
        routes[2].ndcg.push(ndcg_at_k(&hybrid_ids, &relevant, 10));
        routes[2].mrr.push(mrr(&hybrid_ids, &relevant));
        routes[2].p5.push(p_at_5(&hybrid_ids, &relevant));

        let _ = q_no;
        println!("  Q{q_no:02} [{:?}] {}", q.relevant, q.query);
    }

    fn avg(v: &[f64]) -> f64 {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    }

    println!(
        "\n===== RAG 检索质量对比（种子集 {} 条事实 / {} 条查询）=====",
        dataset.facts.len(),
        dataset.queries.len()
    );
    println!(
        "{:<22} {:>10} {:>10} {:>10}",
        "路线", "NDCG@10", "MRR", "P@5"
    );
    for r in &routes {
        println!(
            "{:<22} {:>10.3} {:>10.3} {:>10.3}",
            r.name,
            avg(&r.ndcg),
            avg(&r.mrr),
            avg(&r.p5)
        );
    }
    println!("-----");
    println!(
        "混合 vs 纯向量 NDCG 提升: {:+.3}；混合 vs FTS NDCG 提升: {:+.3}",
        avg(&routes[2].ndcg) - avg(&routes[0].ndcg),
        avg(&routes[2].ndcg) - avg(&routes[1].ndcg)
    );
    println!("注：『混合+Rerank』需先下载本地重排模型（bge-reranker-base）后另行评测；当前内置嵌入为英语向 all-MiniLM-L6-v2，中文查询预期由 FTS/混合腿补足。");
    // The only assertion guardrail: hybrid must not be significantly worse than a single leg (a >0.15 NDCG drop counts as regression)
    assert!(
        avg(&routes[2].ndcg) + 0.15 >= avg(&routes[0].ndcg),
        "混合检索不应明显退化于纯向量"
    );
    assert!(
        avg(&routes[2].ndcg) + 0.15 >= avg(&routes[1].ndcg),
        "混合检索不应明显退化于 FTS"
    );
}
