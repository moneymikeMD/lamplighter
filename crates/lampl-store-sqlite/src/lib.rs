// SPDX-License-Identifier: Apache-2.0

//! Embedded SQLite store for lamplighter: FTS5 bm25 plus sqlite-vec cosine, fused with the shared `lampl-core` ranking.
//! Opened only on an explicit path; this crate never chooses a location.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Once;

use chrono::{DateTime, Utc};
use lampl_core::{
    EMBEDDING_DIM, Floor, Link, LinkType, Memory, NewMemory, RRF_K, fts_coverage, idf,
    passes_floor, query_terms, rrf_fuse,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

const CANDIDATES: i64 = 50;
const SCHEMA_VERSION: i64 = 1;
const MEMORY_COLUMNS: &str = "m.id, m.memory_type, m.title, m.body, m.tags, m.project, m.importance, m.created_at, m.updated_at, m.embedding_model, m.embedding_digest";
const ACTIVE: &str = "NOT EXISTS (SELECT 1 FROM links s WHERE s.to_id = m.id AND s.link_type = 'SUPERSEDES') AND (?1 IS NULL OR m.project = ?1)";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("stored value invalid: {0}")]
    Corrupt(String),
    #[error("embedding has {got} dimensions, expected {EMBEDDING_DIM}")]
    BadDimension { got: usize },
    #[error("memory {0} not found")]
    NotFound(Uuid),
    #[error("a memory cannot link to itself")]
    SelfLink,
}

impl From<lampl_core::ParseError> for StoreError {
    fn from(e: lampl_core::ParseError) -> Self {
        StoreError::Corrupt(e.to_string())
    }
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

#[derive(Debug)]
pub struct SqliteStore {
    conn: Connection,
}

#[allow(unsafe_code)]
fn register_vec_extension() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::ffi::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::ffi::c_int,
        >(
            sqlite_vec::sqlite3_vec_init as *const ()
        )));
    });
}

fn vec_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn check_dim(vector: &[f32]) -> Result<(), StoreError> {
    if vector.len() == EMBEDDING_DIM {
        Ok(())
    } else {
        Err(StoreError::BadDimension { got: vector.len() })
    }
}

fn parse_time(s: &str) -> Result<DateTime<Utc>, StoreError> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| StoreError::Corrupt(format!("timestamp `{s}`: {e}")))
}

fn memory_from_row(row: &Row<'_>) -> Result<Memory, StoreError> {
    let id: String = row.get(0)?;
    let tags: String = row.get(4)?;
    Ok(Memory {
        id: Uuid::parse_str(&id).map_err(|e| StoreError::Corrupt(format!("id `{id}`: {e}")))?,
        memory_type: row.get::<_, String>(1)?.parse()?,
        title: row.get(2)?,
        body: row.get(3)?,
        tags: parse_tags(&tags)?,
        project: row.get(5)?,
        importance: row.get(6)?,
        created_at: parse_time(&row.get::<_, String>(7)?)?,
        updated_at: parse_time(&row.get::<_, String>(8)?)?,
        embedding_model: row.get(9)?,
        embedding_digest: row.get(10)?,
    })
}

fn parse_tags(s: &str) -> Result<Vec<String>, StoreError> {
    s.split('\u{1f}')
        .filter(|t| !t.is_empty())
        .map(|t| Ok(t.to_owned()))
        .collect()
}

fn join_tags(tags: &[String]) -> String {
    tags.join("\u{1f}")
}

fn fts_match(terms: &[String]) -> String {
    terms
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

impl SqliteStore {
    /// Opens (creating if absent) the database at `path` and applies the schema. The caller decides the path; nothing here falls back to a default.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        register_vec_extension();
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        register_vec_extension();
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL, applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')));
             INSERT INTO schema_version (version) SELECT {SCHEMA_VERSION} WHERE NOT EXISTS (SELECT 1 FROM schema_version);
             CREATE TABLE IF NOT EXISTS memories (
                 rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                 id TEXT NOT NULL UNIQUE,
                 memory_type TEXT NOT NULL CHECK (memory_type IN ('fact','decision','problem','fix','procedure','gotcha')),
                 title TEXT NOT NULL,
                 body TEXT NOT NULL,
                 tags TEXT NOT NULL DEFAULT '',
                 project TEXT NOT NULL,
                 importance REAL NOT NULL DEFAULT 0.5 CHECK (importance >= 0 AND importance <= 1),
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 embedding_model TEXT,
                 embedding_digest TEXT
             );
             CREATE INDEX IF NOT EXISTS memories_project_idx ON memories (project);
             CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(title, body, tokenize = 'porter unicode61');
             CREATE VIRTUAL TABLE IF NOT EXISTS memories_vec USING vec0(embedding float[{EMBEDDING_DIM}] distance_metric=cosine);
             CREATE TABLE IF NOT EXISTS links (
                 from_id TEXT NOT NULL REFERENCES memories (id) ON DELETE CASCADE,
                 to_id TEXT NOT NULL REFERENCES memories (id) ON DELETE CASCADE,
                 link_type TEXT NOT NULL CHECK (link_type IN ('SUPERSEDES','SOLVES','CAUSES','CONTRADICTS')),
                 strength REAL NOT NULL DEFAULT 1.0 CHECK (strength >= 0 AND strength <= 1),
                 created_at TEXT NOT NULL,
                 PRIMARY KEY (from_id, to_id, link_type)
             );
             CREATE INDEX IF NOT EXISTS links_to_idx ON links (to_id, link_type);"
        ))?;
        Ok(SqliteStore { conn })
    }

    pub fn schema_version(&self) -> Result<i64, StoreError> {
        Ok(self.conn.query_row(
            "SELECT coalesce(max(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn vec_version(&self) -> Result<String, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT vec_version()", [], |r| r.get(0))?)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn store(&mut self, new: NewMemory) -> Result<Memory, StoreError> {
        if let Some(e) = &new.embedding {
            check_dim(&e.vector)?;
        }
        let id = Uuid::now_v7();
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        let importance = new.importance.clamp(0.0, 1.0);
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO memories (id, memory_type, title, body, tags, project, importance, created_at, updated_at, embedding_model, embedding_digest) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, ?10)",
            params![
                id.to_string(),
                new.memory_type.as_str(),
                new.title,
                new.body,
                join_tags(&new.tags),
                new.project,
                importance,
                now,
                new.embedding.as_ref().map(|e| e.model_id.clone()),
                new.embedding.as_ref().map(|e| e.model_digest.clone()),
            ],
        )?;
        let rowid = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO memories_fts (rowid, title, body) VALUES (?1, ?2, ?3)",
            params![rowid, new.title, new.body],
        )?;
        if let Some(e) = &new.embedding {
            tx.execute(
                "INSERT INTO memories_vec (rowid, embedding) VALUES (?1, ?2)",
                params![rowid, vec_blob(&e.vector)],
            )?;
        }
        tx.commit()?;
        self.get(id)?.ok_or(StoreError::NotFound(id))
    }

    /// Fetches by id regardless of supersession.
    pub fn get(&self, id: Uuid) -> Result<Option<Memory>, StoreError> {
        let sql = format!("SELECT {MEMORY_COLUMNS} FROM memories m WHERE m.id = ?1");
        self.conn
            .query_row(&sql, [id.to_string()], |r| Ok(memory_from_row(r)))
            .optional()?
            .transpose()
    }

    pub fn link(
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
            if self.get(id)?.is_none() {
                return Err(StoreError::NotFound(id));
            }
        }
        let strength = strength.clamp(0.0, 1.0);
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        self.conn.execute(
            "INSERT INTO links (from_id, to_id, link_type, strength, created_at) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (from_id, to_id, link_type) DO UPDATE SET strength = excluded.strength",
            params![from_id.to_string(), to_id.to_string(), link_type.as_str(), strength, now],
        )?;
        let created: String = self.conn.query_row(
            "SELECT created_at FROM links WHERE from_id = ?1 AND to_id = ?2 AND link_type = ?3",
            params![from_id.to_string(), to_id.to_string(), link_type.as_str()],
            |r| r.get(0),
        )?;
        Ok(Link {
            from_id,
            to_id,
            link_type,
            strength,
            created_at: parse_time(&created)?,
        })
    }

    /// Lists linked memories in both directions, including superseded ones.
    pub fn related(&self, id: Uuid) -> Result<Vec<Related>, StoreError> {
        let sql = format!(
            "SELECT {MEMORY_COLUMNS}, l.from_id, l.to_id, l.link_type, l.strength, l.created_at \
             FROM links l JOIN memories m ON m.id = CASE WHEN l.from_id = ?1 THEN l.to_id ELSE l.from_id END \
             WHERE l.from_id = ?1 OR l.to_id = ?1 ORDER BY l.created_at, l.link_type"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([id.to_string()], |row| {
            let from: String = row.get(11)?;
            let to: String = row.get(12)?;
            let link_type: String = row.get(13)?;
            let strength: f32 = row.get(14)?;
            let created: String = row.get(15)?;
            Ok((memory_from_row(row), from, to, link_type, strength, created))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (memory, from, to, link_type, strength, created) = r?;
            let parse = |s: &str| {
                Uuid::parse_str(s).map_err(|e| StoreError::Corrupt(format!("id `{s}`: {e}")))
            };
            let from_id = parse(&from)?;
            out.push(Related {
                direction: if from_id == id {
                    Direction::Outgoing
                } else {
                    Direction::Incoming
                },
                link: Link {
                    from_id,
                    to_id: parse(&to)?,
                    link_type: link_type.parse()?,
                    strength,
                    created_at: parse_time(&created)?,
                },
                memory: memory?,
            });
        }
        Ok(out)
    }

    /// Hybrid recall: FTS5 bm25 and sqlite-vec cosine candidates fused by reciprocal rank, superseded memories excluded, then the relevance floor.
    pub fn recall(&self, q: &RecallQuery) -> Result<Vec<RecallHit>, StoreError> {
        let terms = query_terms(&q.text);
        if terms.is_empty() && q.embedding.is_none() {
            return Ok(Vec::new());
        }
        if let Some(v) = &q.embedding {
            check_dim(v)?;
        }

        let mut rowid_ids: HashMap<i64, Uuid> = HashMap::new();
        let mut fts_ids: Vec<Uuid> = Vec::new();
        if !terms.is_empty() {
            let sql = format!(
                "SELECT m.rowid, m.id FROM memories_fts f JOIN memories m ON m.rowid = f.rowid \
                 WHERE memories_fts MATCH ?2 AND {ACTIVE} ORDER BY bm25(memories_fts), m.rowid LIMIT ?3"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params![q.project, fts_match(&terms), CANDIDATES], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?;
            for r in rows {
                let (rowid, id) = r?;
                let id = Uuid::parse_str(&id)
                    .map_err(|e| StoreError::Corrupt(format!("id `{id}`: {e}")))?;
                rowid_ids.insert(rowid, id);
                fts_ids.push(id);
            }
        }

        let mut cosines: HashMap<Uuid, f64> = HashMap::new();
        let mut vec_ids: Vec<Uuid> = Vec::new();
        if let Some(v) = &q.embedding {
            let sql = format!(
                "SELECT m.id, vec_distance_cosine(v.embedding, ?2) AS d FROM memories m \
                 JOIN memories_vec v ON v.rowid = m.rowid WHERE {ACTIVE} ORDER BY d, m.rowid LIMIT ?3"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params![q.project, vec_blob(v), CANDIDATES], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
            })?;
            for r in rows {
                let (id, d) = r?;
                let id = Uuid::parse_str(&id)
                    .map_err(|e| StoreError::Corrupt(format!("id `{id}`: {e}")))?;
                cosines.insert(id, 1.0 - d);
                vec_ids.push(id);
            }
        }

        let fused = rrf_fuse(&[fts_ids, vec_ids], RRF_K);
        if fused.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = fused.iter().map(|(id, _)| *id).collect();
        let coverage = self.coverage(&terms, &ids, q.project.as_deref())?;

        let mut hits = Vec::new();
        for (id, score) in fused {
            let cosine = cosines.get(&id).copied();
            let cov = coverage.get(&id).copied().unwrap_or(0.0);
            if !passes_floor(q.floor, cosine, cov) {
                continue;
            }
            if let Some(memory) = self.get(id)? {
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

    fn coverage(
        &self,
        terms: &[String],
        ids: &[Uuid],
        project: Option<&str>,
    ) -> Result<HashMap<Uuid, f64>, StoreError> {
        if terms.is_empty() {
            return Ok(HashMap::new());
        }
        let total: i64 = self.conn.query_row(
            "SELECT count(*) FROM memories WHERE ?1 IS NULL OR project = ?1",
            [project],
            |r| r.get(0),
        )?;
        let term_ids_sql = "SELECT m.id FROM memories_fts f JOIN memories m ON m.rowid = f.rowid \
                            WHERE memories_fts MATCH ?2 AND (?1 IS NULL OR m.project = ?1)";
        let mut term_idf = Vec::new();
        let mut matches: HashMap<Uuid, HashSet<String>> = HashMap::new();
        let id_set: HashSet<Uuid> = ids.iter().copied().collect();
        for term in terms {
            let mut stmt = self.conn.prepare(term_ids_sql)?;
            let rows = stmt.query_map(
                params![project, fts_match(std::slice::from_ref(term))],
                |r| r.get::<_, String>(0),
            )?;
            let mut df = 0u64;
            for r in rows {
                df += 1;
                let id = r?;
                let id = Uuid::parse_str(&id)
                    .map_err(|e| StoreError::Corrupt(format!("id `{id}`: {e}")))?;
                if id_set.contains(&id) {
                    matches.entry(id).or_default().insert(term.clone());
                }
            }
            term_idf.push((term.clone(), idf(total.unsigned_abs(), df)));
        }
        Ok(matches
            .into_iter()
            .map(|(id, set)| (id, fts_coverage(&term_idf, &set)))
            .collect())
    }
}
