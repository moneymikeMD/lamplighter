// SPDX-License-Identifier: Apache-2.0

//! Background embedding: a plain tokio task that claims unembedded rows under a short lease, embeds outside any transaction, then writes the vectors in a second short transaction.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use lampl_embed::EmbedError;
use pgvector::HalfVector;
use sqlx::Row;
use uuid::Uuid;

use super::AppState;

const BATCH: i64 = 16;
/// How long a claimed row stays reserved; a worker that dies mid-batch leaves rows reclaimable after this.
const LEASE: Duration = Duration::from_secs(300);
const PROBE_EVERY: Duration = Duration::from_secs(60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const RETRY_FAILED_EVERY: Duration = Duration::from_secs(600);
/// Longest embedder error kept per failed row; `/health` must stay small and never echo memory text back.
const MAX_ERROR_CHARS: usize = 200;

/// Why a row was last skipped by the embedder, and when.
#[derive(Debug, Clone)]
pub(crate) struct FailedRow {
    pub(crate) error: String,
    pub(crate) failed_at: DateTime<Utc>,
}

fn capped(error: &str) -> String {
    match error.char_indices().nth(MAX_ERROR_CHARS) {
        Some((at, _)) => format!("{}…", &error[..at]),
        None => error.to_owned(),
    }
}

impl AppState {
    /// When the worker next clears the failed set so those rows are tried again.
    pub(crate) fn next_retry_at(&self) -> DateTime<Utc> {
        let since = self
            .last_retry
            .lock()
            .map_or(Duration::ZERO, |t| t.elapsed());
        let left = RETRY_FAILED_EVERY.saturating_sub(since);
        Utc::now() + chrono::Duration::from_std(left).unwrap_or_default()
    }

    /// Clears the failed set so the next sweep tries those rows again, and restarts the retry clock; returns how many rows were cleared.
    pub(crate) fn retry_failed(&self) -> usize {
        let n = self.failed.lock().map_or(0, |mut f| {
            let n = f.len();
            f.clear();
            n
        });
        if let Ok(mut t) = self.last_retry.lock() {
            *t = Instant::now();
        }
        n
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("embedder error: {0}")]
    Embed(#[from] EmbedError),
}

/// Embeds on the blocking pool and records the outcome in the health status.
pub(crate) async fn embed_texts(
    state: &AppState,
    texts: Vec<String>,
) -> Result<Vec<Vec<f32>>, EmbedError> {
    let embedder = Arc::clone(&state.embedder);
    let result = tokio::task::spawn_blocking(move || {
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        embedder.embed(&refs)
    })
    .await
    .unwrap_or_else(|e| Err(EmbedError::Embed(format!("embed task failed: {e}"))));
    state.embed_status.record(&result);
    result
}

struct Batch {
    embedded: usize,
    failed: usize,
}

/// Embeds one batch of rows that have no vector from the current model; returns how many were written. A row the embedder rejects is skipped and counted in `/health`, never blocking later rows.
pub async fn embed_pending(state: &AppState) -> Result<usize, WorkerError> {
    Ok(embed_batch(state).await?.embedded)
}

async fn release_claims(state: &AppState, ids: &[Uuid]) {
    if let Err(e) = sqlx::query("UPDATE memories SET embed_claimed_until = NULL WHERE id = ANY($1)")
        .bind(ids)
        .execute(state.store.pool())
        .await
    {
        tracing::warn!(error = %e, "could not release embedding claims; they lapse with the lease");
    }
}

async fn embed_batch(state: &AppState) -> Result<Batch, WorkerError> {
    let skip: Vec<Uuid> = state
        .failed
        .lock()
        .map(|f| f.keys().copied().collect())
        .unwrap_or_default();
    let rows = sqlx::query(
        "UPDATE memories SET embed_claimed_until = now() + make_interval(secs => $4) \
         WHERE id IN (SELECT id FROM memories \
             WHERE (embedding IS NULL OR embedding_digest IS DISTINCT FROM $1) AND NOT (id = ANY($3)) \
               AND (embed_claimed_until IS NULL OR embed_claimed_until < now()) \
             ORDER BY id LIMIT $2 FOR UPDATE SKIP LOCKED) \
         RETURNING id, title, body",
    )
    .bind(state.embedder.model_digest())
    .bind(BATCH)
    .bind(&skip)
    .bind(LEASE.as_secs_f64())
    .fetch_all(state.store.pool())
    .await?;
    if rows.is_empty() {
        return Ok(Batch {
            embedded: 0,
            failed: 0,
        });
    }
    let mut ids: Vec<Uuid> = Vec::with_capacity(rows.len());
    let mut texts = Vec::with_capacity(rows.len());
    for row in &rows {
        ids.push(row.try_get("id")?);
        let (title, body): (String, String) = (row.try_get("title")?, row.try_get("body")?);
        texts.push(format!("{title}\n\n{body}"));
    }
    match embed_and_write(state, &ids, texts).await {
        Ok(batch) => Ok(batch),
        Err(e) => {
            release_claims(state, &ids).await;
            Err(e)
        }
    }
}

async fn embed_and_write(
    state: &AppState,
    ids: &[Uuid],
    texts: Vec<String>,
) -> Result<Batch, WorkerError> {
    let whole = embed_texts(state, texts.clone()).await;
    let results: Vec<Result<Vec<f32>, String>> = match whole {
        Ok(v) if v.len() == ids.len() => v.into_iter().map(Ok).collect(),
        Ok(v) => {
            return Err(EmbedError::Embed(format!(
                "embedder returned {} vectors for {} inputs",
                v.len(),
                ids.len()
            ))
            .into());
        }
        Err(e) => {
            probe(state).await;
            if !state.embed_status.is_ok() {
                return Err(e.into());
            }
            let mut out = Vec::with_capacity(texts.len());
            for t in texts {
                out.push(match embed_texts(state, vec![t]).await {
                    Ok(mut v) => v
                        .pop()
                        .ok_or_else(|| "embedder returned no vector".to_owned()),
                    Err(e) => Err(e.to_string()),
                });
            }
            out
        }
    };
    let mut batch = Batch {
        embedded: 0,
        failed: 0,
    };
    let mut tx = state.store.pool().begin().await?;
    for (id, vector) in ids.iter().zip(&results) {
        let vector = vector.as_ref().map_err(String::clone).and_then(|v| {
            if v.len() == lampl_core::EMBEDDING_DIM {
                Ok(v)
            } else {
                Err(format!(
                    "embedder returned {} dimensions, expected {}",
                    v.len(),
                    lampl_core::EMBEDDING_DIM
                ))
            }
        });
        let vector = match vector {
            Ok(v) => v,
            Err(error) => {
                let error = capped(&error);
                tracing::warn!(%id, %error, "row could not be embedded; skipping it");
                if let Ok(mut f) = state.failed.lock() {
                    f.insert(
                        *id,
                        FailedRow {
                            error,
                            failed_at: Utc::now(),
                        },
                    );
                }
                sqlx::query("UPDATE memories SET embed_claimed_until = NULL WHERE id = $1")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                batch.failed += 1;
                continue;
            }
        };
        sqlx::query(
            "UPDATE memories SET embedding = $2, embedding_model = $3, embedding_digest = $4, embed_claimed_until = NULL WHERE id = $1",
        )
        .bind(id)
        .bind(HalfVector::from_f32_slice(vector))
        .bind(state.embedder.model_id())
        .bind(state.embedder.model_digest())
        .execute(&mut *tx)
        .await?;
        if let Ok(mut f) = state.failed.lock() {
            f.remove(id);
        }
        batch.embedded += 1;
    }
    tx.commit().await?;
    Ok(batch)
}

/// Embeds a short text so `/health` reflects the embedder even when there is nothing to embed.
pub async fn probe(state: &AppState) {
    let _ = embed_texts(state, vec!["health probe".to_owned()]).await;
}

/// Like `probe`, but a hung embedder counts as a failure after a few seconds instead of hanging `/health`.
pub async fn probe_bounded(state: &AppState) {
    if tokio::time::timeout(PROBE_TIMEOUT, probe(state))
        .await
        .is_err()
    {
        state
            .embed_status
            .record::<()>(&Err(EmbedError::Embed("probe timed out".to_owned())));
    }
}

pub(crate) async fn run(state: Arc<AppState>, interval: Duration) {
    let mut last_probe = Instant::now();
    loop {
        match embed_batch(&state).await {
            Ok(b) if b.embedded + b.failed > 0 => {
                tracing::info!(embedded = b.embedded, failed = b.failed, "embedding sweep");
                continue;
            }
            Ok(_) => {
                let due = state
                    .last_retry
                    .lock()
                    .is_ok_and(|t| t.elapsed() >= RETRY_FAILED_EVERY);
                if due {
                    state.retry_failed();
                }
                if !state.embed_status.is_ok() || last_probe.elapsed() >= PROBE_EVERY {
                    probe(&state).await;
                    last_probe = Instant::now();
                }
            }
            Err(e) => tracing::warn!(error = %e, "background embedding failed"),
        }
        tokio::time::sleep(interval).await;
    }
}
