// SPDX-License-Identifier: Apache-2.0

//! Postgres and pgvector store for lamplighter: typed memories, supersession and hybrid recall.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use lampl_core::{
    EMBEDDING_DIM, Floor, Link, LinkType, Memory, NewMemory, RRF_K, fts_coverage, idf,
    passes_floor, query_terms, rrf_fuse,
};
use pgvector::HalfVector;
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::{AssertSqlSafe, Row};
use uuid::Uuid;

// The interpolated SQL fragments are compile-time constants; no caller input reaches a query string.
const CANDIDATES: i64 = 50;
const MEMORY_COLUMNS: &str = "id, memory_type, title, body, tags, project, importance, created_at, updated_at, embedding_model, embedding_digest";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("stored value invalid: {0}")]
    Corrupt(#[from] lampl_core::ParseError),
    #[error("embedding has {got} dimensions, expected {EMBEDDING_DIM}")]
    BadDimension { got: usize },
    #[error("memory {0} not found")]
    NotFound(Uuid),
    #[error("a memory cannot link to itself")]
    SelfLink,
}

#[derive(Debug, Clone)]
pub struct RecallQuery {
    pub text: String,
    pub embedding: Option<Vec<f32>>,
    pub project: Option<String>,
    pub limit: usize,
    pub floor: Floor,
}

impl RecallQuery {
    pub fn new(text: impl Into<String>) -> Self {
        RecallQuery {
            text: text.into(),
            embedding: None,
            project: None,
            limit: 10,
            floor: Floor::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecallHit {
    pub memory: Memory,
    pub score: f64,
    pub cosine: Option<f64>,
    pub coverage: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Outgoing,
    Incoming,
}

#[derive(Debug, Clone)]
pub struct Related {
    pub direction: Direction,
    pub link: Link,
    pub memory: Memory,
}

#[derive(Debug, Clone)]
pub struct PgStore {
    pool: PgPool,
}

fn memory_from_row(row: &PgRow) -> Result<Memory, StoreError> {
    Ok(Memory {
        id: row.try_get("id")?,
        memory_type: row.try_get::<String, _>("memory_type")?.parse()?,
        title: row.try_get("title")?,
        body: row.try_get("body")?,
        tags: row.try_get("tags")?,
        project: row.try_get("project")?,
        importance: row.try_get("importance")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        embedding_model: row.try_get("embedding_model")?,
        embedding_digest: row.try_get("embedding_digest")?,
    })
}

fn check_dim(vector: &[f32]) -> Result<(), StoreError> {
    if vector.len() == EMBEDDING_DIM {
        Ok(())
    } else {
        Err(StoreError::BadDimension { got: vector.len() })
    }
}

impl PgStore {
    /// Connects and applies pending migrations. Migrations are expand-contract so a client one minor behind keeps working.
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new().max_connections(8).connect(url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(PgStore { pool })
    }

    pub async fn schema_version(&self) -> Result<i32, StoreError> {
        let v: Option<i32> = sqlx::query_scalar("SELECT max(version) FROM schema_version")
            .fetch_one(&self.pool)
            .await?;
        Ok(v.unwrap_or(0))
    }

    pub async fn store(&self, new: NewMemory) -> Result<Memory, StoreError> {
        let (vector, model, digest) = match &new.embedding {
            Some(e) => {
                check_dim(&e.vector)?;
                (
                    Some(HalfVector::from_f32_slice(&e.vector)),
                    Some(e.model_id.clone()),
                    Some(e.model_digest.clone()),
                )
            }
            None => (None, None, None),
        };
        let sql = format!(
            "INSERT INTO memories (id, memory_type, title, body, tags, project, importance, embedding, embedding_model, embedding_digest) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING {MEMORY_COLUMNS}"
        );
        let row = sqlx::query(AssertSqlSafe(sql))
            .bind(Uuid::now_v7())
            .bind(new.memory_type.as_str())
            .bind(&new.title)
            .bind(&new.body)
            .bind(&new.tags)
            .bind(&new.project)
            .bind(new.importance.clamp(0.0, 1.0))
            .bind(vector)
            .bind(model)
            .bind(digest)
            .fetch_one(&self.pool)
            .await?;
        memory_from_row(&row)
    }

    /// Fetches by id regardless of supersession.
    pub async fn get(&self, id: Uuid) -> Result<Option<Memory>, StoreError> {
        let sql = format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id = $1");
        let row = sqlx::query(AssertSqlSafe(sql))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(memory_from_row).transpose()
    }

    pub async fn link(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        link_type: LinkType,
        strength: f32,
    ) -> Result<Link, StoreError> {
        if from_id == to_id {
            return Err(StoreError::SelfLink);
        }
        for id in [from_id, to_id] {
            if self.get(id).await?.is_none() {
                return Err(StoreError::NotFound(id));
            }
        }
        let row = sqlx::query(
            "INSERT INTO links (from_id, to_id, link_type, strength) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (from_id, to_id, link_type) DO UPDATE SET strength = EXCLUDED.strength \
             RETURNING created_at",
        )
        .bind(from_id)
        .bind(to_id)
        .bind(link_type.as_str())
        .bind(strength.clamp(0.0, 1.0))
        .fetch_one(&self.pool)
        .await?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        Ok(Link {
            from_id,
            to_id,
            link_type,
            strength: strength.clamp(0.0, 1.0),
            created_at,
        })
    }

    /// Lists linked memories in both directions, including superseded ones.
    pub async fn related(&self, id: Uuid) -> Result<Vec<Related>, StoreError> {
        let cols = MEMORY_COLUMNS
            .split(", ")
            .map(|c| format!("m.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {cols}, l.from_id, l.to_id, l.link_type, l.strength, l.created_at AS link_created_at \
             FROM links l JOIN memories m ON m.id = CASE WHEN l.from_id = $1 THEN l.to_id ELSE l.from_id END \
             WHERE l.from_id = $1 OR l.to_id = $1 ORDER BY l.created_at, l.link_type"
        );
        let rows = sqlx::query(AssertSqlSafe(sql))
            .bind(id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                let from_id: Uuid = row.try_get("from_id")?;
                Ok(Related {
                    direction: if from_id == id {
                        Direction::Outgoing
                    } else {
                        Direction::Incoming
                    },
                    link: Link {
                        from_id,
                        to_id: row.try_get("to_id")?,
                        link_type: row.try_get::<String, _>("link_type")?.parse()?,
                        strength: row.try_get("strength")?,
                        created_at: row.try_get("link_created_at")?,
                    },
                    memory: memory_from_row(row)?,
                })
            })
            .collect()
    }

    /// Hybrid recall: full-text and cosine candidates fused by reciprocal rank, superseded memories excluded, then the relevance floor.
    pub async fn recall(&self, q: &RecallQuery) -> Result<Vec<RecallHit>, StoreError> {
        let terms = query_terms(&q.text);
        if terms.is_empty() && q.embedding.is_none() {
            return Ok(Vec::new());
        }
        let vector = match &q.embedding {
            Some(v) => {
                check_dim(v)?;
                Some(HalfVector::from_f32_slice(v))
            }
            None => None,
        };
        let active = "NOT EXISTS (SELECT 1 FROM links s WHERE s.to_id = m.id AND s.link_type = 'SUPERSEDES') \
                      AND ($1::text IS NULL OR m.project = $1)";

        let fts_ids: Vec<Uuid> = if terms.is_empty() {
            Vec::new()
        } else {
            let sql = format!(
                "SELECT m.id FROM memories m, \
                 (SELECT replace(plainto_tsquery('english', $2)::text, '&', '|')::tsquery AS tq) q \
                 WHERE m.tsv @@ q.tq AND {active} ORDER BY ts_rank_cd(m.tsv, q.tq) DESC, m.id LIMIT $3"
            );
            sqlx::query_scalar(AssertSqlSafe(sql))
                .bind(&q.project)
                .bind(terms.join(" "))
                .bind(CANDIDATES)
                .fetch_all(&self.pool)
                .await?
        };

        let mut cosines: HashMap<Uuid, f64> = HashMap::new();
        let mut vec_ids: Vec<Uuid> = Vec::new();
        if let Some(v) = &vector {
            let sql = format!(
                "SELECT m.id, 1 - (m.embedding <=> $2) AS cos FROM memories m \
                 WHERE m.embedding IS NOT NULL AND {active} ORDER BY m.embedding <=> $2, m.id LIMIT $3"
            );
            let rows = sqlx::query(AssertSqlSafe(sql))
                .bind(&q.project)
                .bind(v)
                .bind(CANDIDATES)
                .fetch_all(&self.pool)
                .await?;
            for row in &rows {
                let id: Uuid = row.try_get("id")?;
                cosines.insert(id, row.try_get("cos")?);
                vec_ids.push(id);
            }
        }

        let fused = rrf_fuse(&[fts_ids, vec_ids], RRF_K);
        if fused.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = fused.iter().map(|(id, _)| *id).collect();

        let coverage = self.coverage(&terms, &ids, q.project.as_deref()).await?;

        let sql = format!("SELECT {MEMORY_COLUMNS} FROM memories WHERE id = ANY($1)");
        let rows = sqlx::query(AssertSqlSafe(sql))
            .bind(&ids)
            .fetch_all(&self.pool)
            .await?;
        let mut by_id: HashMap<Uuid, Memory> = HashMap::new();
        for row in &rows {
            let m = memory_from_row(row)?;
            by_id.insert(m.id, m);
        }

        let mut hits = Vec::new();
        for (id, score) in fused {
            let cosine = cosines.get(&id).copied();
            let cov = coverage.get(&id).copied().unwrap_or(0.0);
            if !passes_floor(q.floor, cosine, cov) {
                continue;
            }
            if let Some(memory) = by_id.remove(&id) {
                hits.push(RecallHit {
                    memory,
                    score,
                    cosine,
                    coverage: cov,
                });
            }
            if hits.len() >= q.limit {
                break;
            }
        }
        Ok(hits)
    }

    async fn coverage(
        &self,
        terms: &[String],
        ids: &[Uuid],
        project: Option<&str>,
    ) -> Result<HashMap<Uuid, f64>, StoreError> {
        if terms.is_empty() {
            return Ok(HashMap::new());
        }
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM memories WHERE $1::text IS NULL OR project = $1",
        )
        .bind(project)
        .fetch_one(&self.pool)
        .await?;
        let df_rows = sqlx::query(
            "SELECT t.term, count(m.id) AS df FROM unnest($2::text[]) AS t(term) \
             LEFT JOIN memories m ON m.tsv @@ plainto_tsquery('english', t.term) AND ($1::text IS NULL OR m.project = $1) \
             GROUP BY t.term",
        )
        .bind(project)
        .bind(terms)
        .fetch_all(&self.pool)
        .await?;
        let mut weights: HashMap<String, f64> = HashMap::new();
        for row in &df_rows {
            let term: String = row.try_get("term")?;
            let df: i64 = row.try_get("df")?;
            weights.insert(term, idf(total.unsigned_abs(), df.unsigned_abs()));
        }
        let term_idf: Vec<(String, f64)> = terms
            .iter()
            .map(|t| (t.clone(), weights.get(t).copied().unwrap_or(0.0)))
            .collect();

        let match_rows = sqlx::query(
            "SELECT m.id, array_agg(t.term) AS matched FROM memories m \
             JOIN unnest($1::text[]) AS t(term) ON m.tsv @@ plainto_tsquery('english', t.term) \
             WHERE m.id = ANY($2) GROUP BY m.id",
        )
        .bind(terms)
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;
        let mut out = HashMap::new();
        for row in &match_rows {
            let id: Uuid = row.try_get("id")?;
            let matched: Vec<String> = row.try_get("matched")?;
            let set: HashSet<String> = matched.into_iter().collect();
            out.insert(id, fts_coverage(&term_idf, &set));
        }
        Ok(out)
    }
}
