// SPDX-License-Identifier: Apache-2.0

use std::io;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Extension, FromRequest, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use lampl_core::{CoverageNorm, Floor, query_embed_text};
use lampl_core::{Embedding, Link, LinkType, Memory, MemoryType, NewMemory};
use lampl_store_pg::{Direction, RecallQuery};
use pgvector::HalfVector;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::error::DatabaseError;
use uuid::Uuid;

use super::auth::ClientName;
use super::error::ApiError;
use super::worker::embed_texts;
use super::{API_VERSION, AppState};

const PAGE: i64 = 500;
const MAX_LIMIT: usize = 100;
const LIST_MAX: usize = 1000;

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub(crate) struct Json<T>(T);

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub(crate) struct MemoryDto {
    id: Uuid,
    memory_type: String,
    title: String,
    body: String,
    tags: Vec<String>,
    project: String,
    importance: f32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    embedding_model: Option<String>,
    #[serde(default)]
    embedding_digest: Option<String>,
}

impl From<Memory> for MemoryDto {
    fn from(m: Memory) -> Self {
        MemoryDto {
            id: m.id,
            memory_type: m.memory_type.to_string(),
            title: m.title,
            body: m.body,
            tags: m.tags,
            project: m.project,
            importance: m.importance,
            created_at: m.created_at,
            updated_at: m.updated_at,
            embedding_model: m.embedding_model,
            embedding_digest: m.embedding_digest,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub(crate) struct LinkDto {
    from_id: Uuid,
    to_id: Uuid,
    link_type: String,
    strength: f32,
    created_at: DateTime<Utc>,
}

impl From<Link> for LinkDto {
    fn from(l: Link) -> Self {
        LinkDto {
            from_id: l.from_id,
            to_id: l.to_id,
            link_type: l.link_type.to_string(),
            strength: l.strength,
            created_at: l.created_at,
        }
    }
}

fn parse_type(s: &str) -> Result<MemoryType, ApiError> {
    s.parse()
        .map_err(|e: lampl_core::ParseError| ApiError::bad_request(e.to_string()))
}

fn parse_link_type(s: &str) -> Result<LinkType, ApiError> {
    s.parse()
        .map_err(|e: lampl_core::ParseError| ApiError::bad_request(e.to_string()))
}

#[derive(Debug, Deserialize)]
pub(crate) struct StoreBody {
    memory_type: String,
    title: String,
    body: String,
    #[serde(default)]
    tags: Vec<String>,
    project: String,
    #[serde(default = "default_importance")]
    importance: f32,
}

fn default_importance() -> f32 {
    0.5
}

pub(crate) async fn store_memory(
    State(state): State<Arc<AppState>>,
    Json(b): Json<StoreBody>,
) -> Result<(StatusCode, Json<MemoryDto>), ApiError> {
    if b.title.trim().is_empty() || b.project.trim().is_empty() {
        return Err(ApiError::bad_request("title and project must not be empty"));
    }
    let new = NewMemory {
        memory_type: parse_type(&b.memory_type)?,
        title: b.title,
        body: b.body,
        tags: b.tags,
        project: b.project,
        importance: b.importance,
        embedding: None::<Embedding>,
    };
    let m = state.store.store(new).await?;
    Ok((StatusCode::CREATED, Json(m.into())))
}

pub(crate) async fn get_memory(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<MemoryDto>, ApiError> {
    state
        .store
        .get(id)
        .await?
        .map(|m| Json(m.into()))
        .ok_or_else(ApiError::not_found)
}

#[derive(Debug, Serialize)]
pub(crate) struct DeleteOut {
    id: Uuid,
    links_removed: u64,
}

/// Logs the id and the calling client, never the deleted text.
pub(crate) async fn delete_memory(
    State(state): State<Arc<AppState>>,
    Extension(ClientName(client)): Extension<ClientName>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteOut>, ApiError> {
    let links_removed =
        state.store.delete(id).await?.ok_or_else(|| {
            ApiError::new(StatusCode::NOT_FOUND, format!("memory {id} not found"))
        })?;
    if let Ok(mut f) = state.failed.lock() {
        f.remove(&id);
    }
    tracing::info!(%id, %client, links_removed, "memory deleted");
    Ok(Json(DeleteOut { id, links_removed }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListParams {
    project: Option<String>,
    after: Option<Uuid>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ListOut {
    memories: Vec<MemoryDto>,
    next_after: Option<Uuid>,
}

pub(crate) async fn list_memories(
    State(state): State<Arc<AppState>>,
    Query(p): Query<ListParams>,
) -> Result<Json<ListOut>, ApiError> {
    let limit = p.limit.unwrap_or(50).clamp(1, LIST_MAX);
    let memories: Vec<MemoryDto> = sqlx::query_as(
        "SELECT id, memory_type, title, body, tags, project, importance, created_at, updated_at, embedding_model, embedding_digest \
         FROM memories WHERE ($1::text IS NULL OR project = $1) AND ($2::uuid IS NULL OR id > $2) ORDER BY id LIMIT $3",
    )
    .bind(&p.project)
    .bind(p.after)
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .fetch_all(state.store.pool())
    .await?;
    let next_after = if memories.len() == limit {
        memories.last().map(|m| m.id)
    } else {
        None
    };
    Ok(Json(ListOut {
        memories,
        next_after,
    }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct RecallBody {
    query: String,
    project: Option<String>,
    limit: Option<usize>,
    /// Past this the query embedding is abandoned and the recall is full-text only.
    embed_timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct HitOut {
    memory: MemoryDto,
    score: f64,
    cosine: Option<f64>,
    coverage: f64,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecallOut {
    mode: &'static str,
    hits: Vec<HitOut>,
}

/// Benchmarking only: `LAMPL_RECALL_FLOOR=off` or `<cosine>,<coverage>` replaces the default floor's thresholds and keeps its coverage norm; unset keeps the shipped default.
fn floor_override() -> Option<Floor> {
    static OVERRIDE: std::sync::OnceLock<Option<Floor>> = std::sync::OnceLock::new();
    *OVERRIDE.get_or_init(|| {
        let v = std::env::var("LAMPL_RECALL_FLOOR").ok()?;
        let v = v.trim();
        if v.eq_ignore_ascii_case("off") {
            return Some(Floor {
                cosine: f64::NEG_INFINITY,
                coverage: 0.0,
                ..Floor::default()
            });
        }
        let (c, v) = v.split_once(',')?;
        Some(Floor {
            cosine: c.trim().parse().ok()?,
            coverage: v.trim().parse().ok()?,
            ..Floor::default()
        })
    })
}

/// Benchmarking only: `LAMPL_RECALL_COVERAGE=plain`, `salient:<k>` or `capped:<k>` changes the coverage the floor tests (see `CoverageNorm`); unset keeps the default `capped:3`.
fn coverage_norm_override() -> Option<CoverageNorm> {
    static OVERRIDE: std::sync::OnceLock<Option<CoverageNorm>> = std::sync::OnceLock::new();
    *OVERRIDE.get_or_init(|| {
        let v = std::env::var("LAMPL_RECALL_COVERAGE").ok()?;
        match v.parse() {
            Ok(n) => Some(n),
            Err(e) => {
                tracing::warn!(error = %e, "LAMPL_RECALL_COVERAGE ignored");
                None
            }
        }
    })
}

pub(crate) async fn recall(
    State(state): State<Arc<AppState>>,
    Json(b): Json<RecallBody>,
) -> Result<Json<RecallOut>, ApiError> {
    let mut q = RecallQuery::new(b.query.clone());
    q.project = b.project;
    q.limit = b.limit.unwrap_or(10).clamp(1, MAX_LIMIT);
    if let Some(f) = floor_override() {
        q.floor = f;
    }
    if let Some(n) = coverage_norm_override() {
        q.floor.coverage_norm = n;
    }
    let embedded = if state.embed_status.is_ok() {
        let embedding = embed_texts(&state, vec![query_embed_text(&b.query)]);
        match b.embed_timeout_ms {
            Some(ms) => tokio::time::timeout(std::time::Duration::from_millis(ms), embedding)
                .await
                .ok()
                .and_then(Result::ok),
            None => embedding.await.ok(),
        }
    } else {
        None
    };
    if let Some(mut v) = embedded {
        q.embedding = v.pop();
        q.embedding_digest = Some(state.embedder.model_digest().to_owned());
    }
    let mode = if q.embedding.is_some() {
        "hybrid"
    } else {
        "full-text"
    };
    let hits = state
        .store
        .recall(&q)
        .await?
        .into_iter()
        .map(|h| HitOut {
            memory: h.memory.into(),
            score: h.score,
            cosine: h.cosine,
            coverage: h.coverage,
        })
        .collect();
    Ok(Json(RecallOut { mode, hits }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct LinkBody {
    from_id: Uuid,
    to_id: Uuid,
    link_type: String,
    #[serde(default = "default_strength")]
    strength: f32,
}

fn default_strength() -> f32 {
    1.0
}

pub(crate) async fn create_link(
    State(state): State<Arc<AppState>>,
    Json(b): Json<LinkBody>,
) -> Result<(StatusCode, Json<LinkDto>), ApiError> {
    let t = parse_link_type(&b.link_type)?;
    let link = state.store.link(b.from_id, b.to_id, t, b.strength).await?;
    Ok((StatusCode::CREATED, Json(link.into())))
}

#[derive(Debug, Serialize)]
pub(crate) struct RelatedOut {
    direction: &'static str,
    link: LinkDto,
    memory: MemoryDto,
}

pub(crate) async fn related(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<RelatedOut>>, ApiError> {
    if state.store.get(id).await?.is_none() {
        return Err(ApiError::not_found());
    }
    let out = state
        .store
        .related(id)
        .await?
        .into_iter()
        .map(|r| RelatedOut {
            direction: match r.direction {
                Direction::Outgoing => "outgoing",
                Direction::Incoming => "incoming",
            },
            link: r.link.into(),
            memory: r.memory.into(),
        })
        .collect();
    Ok(Json(out))
}

const EXPORT_FORMAT: &str = "lampl-export";
const EXPORT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    format: String,
    format_version: u32,
    api_version: u32,
    embed_model: String,
    embed_digest: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct End {
    memories: u64,
    links: u64,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
struct MemoryLine {
    #[serde(flatten)]
    #[sqlx(flatten)]
    memory: MemoryDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vector: Option<Vec<f32>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Line {
    Header(Header),
    Memory(MemoryLine),
    Link(LinkDto),
    End(End),
}

enum Phase {
    Header,
    Memories { after: Option<Uuid>, count: u64 },
    Links { memories: u64 },
    End { memories: u64, links: u64 },
    Done,
}

struct ExportCtx {
    pool: PgPool,
    project: Option<String>,
    vectors: bool,
    model: String,
    digest: String,
}

async fn next_chunk(ctx: &ExportCtx, phase: Phase) -> Result<Option<(Bytes, Phase)>, sqlx::Error> {
    match phase {
        Phase::Header => {
            let line = Line::Header(Header {
                format: EXPORT_FORMAT.to_owned(),
                format_version: EXPORT_FORMAT_VERSION,
                api_version: API_VERSION,
                embed_model: ctx.model.clone(),
                embed_digest: ctx.digest.clone(),
            });
            Ok(Some((
                ndjson(std::iter::once(line)),
                Phase::Memories {
                    after: None,
                    count: 0,
                },
            )))
        }
        Phase::Memories { after, count } => {
            let rows: Vec<MemoryLine> = sqlx::query_as(
                "SELECT id, memory_type, title, body, tags, project, importance, created_at, updated_at, embedding_model, embedding_digest, \
                 CASE WHEN $4 THEN (embedding::vector)::real[] END AS vector \
                 FROM memories WHERE ($1::text IS NULL OR project = $1) AND ($2::uuid IS NULL OR id > $2) ORDER BY id LIMIT $3",
            )
            .bind(ctx.project.as_deref())
            .bind(after)
            .bind(PAGE)
            .bind(ctx.vectors)
            .fetch_all(&ctx.pool)
            .await?;
            let Some(last) = rows.last().map(|m| m.memory.id) else {
                return Box::pin(next_chunk(ctx, Phase::Links { memories: count })).await;
            };
            let count = count + rows.len() as u64;
            let next = if i64::try_from(rows.len()).unwrap_or(0) < PAGE {
                Phase::Links { memories: count }
            } else {
                Phase::Memories {
                    after: Some(last),
                    count,
                }
            };
            Ok(Some((ndjson(rows.into_iter().map(Line::Memory)), next)))
        }
        Phase::Links { memories } => {
            let rows: Vec<LinkDto> = sqlx::query_as(
                "SELECT from_id, to_id, link_type, strength, created_at FROM links \
                 WHERE $1::text IS NULL OR (from_id IN (SELECT id FROM memories WHERE project = $1) \
                   AND to_id IN (SELECT id FROM memories WHERE project = $1)) \
                 ORDER BY created_at, from_id, to_id, link_type",
            )
            .bind(ctx.project.as_deref())
            .fetch_all(&ctx.pool)
            .await?;
            let links = rows.len() as u64;
            let next = Phase::End { memories, links };
            if rows.is_empty() {
                return Box::pin(next_chunk(ctx, next)).await;
            }
            Ok(Some((ndjson(rows.into_iter().map(Line::Link)), next)))
        }
        Phase::End { memories, links } => Ok(Some((
            ndjson(std::iter::once(Line::End(End { memories, links }))),
            Phase::Done,
        ))),
        Phase::Done => Ok(None),
    }
}

fn ndjson(lines: impl Iterator<Item = Line>) -> Bytes {
    let mut out = Vec::new();
    for line in lines {
        if serde_json::to_writer(&mut out, &line).is_ok() {
            out.push(b'\n');
        }
    }
    Bytes::from(out)
}

#[derive(Debug, Deserialize)]
pub(crate) struct ExportParams {
    project: Option<String>,
    #[serde(default = "default_true")]
    vectors: bool,
}

fn default_true() -> bool {
    true
}

pub(crate) async fn export(
    State(state): State<Arc<AppState>>,
    Query(p): Query<ExportParams>,
) -> Response {
    let ctx = Arc::new(ExportCtx {
        pool: state.store.pool().clone(),
        project: p.project,
        vectors: p.vectors,
        model: state.embedder.model_id().to_owned(),
        digest: state.embedder.model_digest().to_owned(),
    });
    let stream = futures_util::stream::unfold(Phase::Header, move |phase| {
        let ctx = Arc::clone(&ctx);
        async move {
            match next_chunk(&ctx, phase).await {
                Ok(Some((bytes, next))) => Some((Ok(bytes), next)),
                Ok(None) => None,
                Err(e) => {
                    tracing::error!(error = %e, "export failed mid-stream");
                    Some((Err(io::Error::other("export aborted")), Phase::Done))
                }
            }
        }
    });
    (
        [(header::CONTENT_TYPE, "application/x-ndjson")],
        Body::from_stream(stream),
    )
        .into_response()
}

#[derive(Debug, Serialize)]
pub(crate) struct ImportOut {
    memories_imported: u64,
    memories_skipped: u64,
    links_imported: u64,
    links_skipped: u64,
}

pub(crate) async fn import(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Json<ImportOut>, ApiError> {
    let text =
        std::str::from_utf8(&body).map_err(|_| ApiError::bad_request("body is not UTF-8"))?;
    let mut memories = Vec::new();
    let mut links = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let line: Line = serde_json::from_str(raw)
            .map_err(|e| ApiError::bad_request(format!("line {}: {e}", i + 1)))?;
        match line {
            Line::Header(_) | Line::End(_) => {}
            Line::Memory(m) => {
                parse_type(&m.memory.memory_type).map_err(|e| {
                    ApiError::bad_request(format!("line {}: {}", i + 1, e.message()))
                })?;
                if let Some(v) = &m.vector
                    && (v.len() != lampl_core::EMBEDDING_DIM
                        || m.memory.embedding_model.is_none()
                        || m.memory.embedding_digest.is_none())
                {
                    return Err(ApiError::bad_request(format!(
                        "line {}: a vector needs {} values plus embedding_model and embedding_digest",
                        i + 1,
                        lampl_core::EMBEDDING_DIM
                    )));
                }
                memories.push(m);
            }
            Line::Link(l) => {
                parse_link_type(&l.link_type).map_err(|e| {
                    ApiError::bad_request(format!("line {}: {}", i + 1, e.message()))
                })?;
                links.push(l);
            }
        }
    }
    let mut out = ImportOut {
        memories_imported: 0,
        memories_skipped: 0,
        links_imported: 0,
        links_skipped: 0,
    };
    let mut tx = state.store.pool().begin().await?;
    for line in &memories {
        let m = &line.memory;
        let vector = line.vector.as_deref().map(HalfVector::from_f32_slice);
        let (model, digest) = if vector.is_some() {
            (m.embedding_model.as_deref(), m.embedding_digest.as_deref())
        } else {
            (None, None)
        };
        let n = sqlx::query(
            "INSERT INTO memories (id, memory_type, title, body, tags, project, importance, created_at, updated_at, embedding, embedding_model, embedding_digest) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) ON CONFLICT (id) DO NOTHING",
        )
        .bind(m.id)
        .bind(&m.memory_type)
        .bind(&m.title)
        .bind(&m.body)
        .bind(&m.tags)
        .bind(&m.project)
        .bind(m.importance.clamp(0.0, 1.0))
        .bind(m.created_at)
        .bind(m.updated_at)
        .bind(vector)
        .bind(model)
        .bind(digest)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        out.memories_imported += n;
        out.memories_skipped += 1 - n;
    }
    for l in &links {
        let res = sqlx::query(
            "INSERT INTO links (from_id, to_id, link_type, strength, created_at) VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT DO NOTHING",
        )
        .bind(l.from_id)
        .bind(l.to_id)
        .bind(&l.link_type)
        .bind(l.strength.clamp(0.0, 1.0))
        .bind(l.created_at)
        .execute(&mut *tx)
        .await;
        match res {
            Ok(r) => {
                out.links_imported += r.rows_affected();
                out.links_skipped += 1 - r.rows_affected();
            }
            Err(e)
                if e.as_database_error()
                    .and_then(DatabaseError::code)
                    .as_deref()
                    == Some("23503") =>
            {
                return Err(ApiError::bad_request("a link references an unknown memory"));
            }
            Err(e) => return Err(e.into()),
        }
    }
    tx.commit().await?;
    Ok(Json(out))
}
