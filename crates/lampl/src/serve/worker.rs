// SPDX-License-Identifier: Apache-2.0

//! Background embedding: a plain tokio task claiming unembedded rows with `FOR UPDATE SKIP LOCKED`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use lampl_embed::EmbedError;
use pgvector::HalfVector;
use sqlx::Row;
use uuid::Uuid;

use super::AppState;

const BATCH: i64 = 16;
const PROBE_EVERY: Duration = Duration::from_secs(60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_FAILED_EVERY: Duration = Duration::from_secs(600);

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

async fn embed_batch(state: &AppState) -> Result<Batch, WorkerError> {
    let skip: Vec<Uuid> = state
        .failed
        .lock()
        .map(|f| f.iter().copied().collect())
        .unwrap_or_default();
    let mut tx = state.store.pool().begin().await?;
    let rows = sqlx::query(
        "SELECT id, title, body FROM memories \
         WHERE (embedding IS NULL OR embedding_digest IS DISTINCT FROM $1) AND NOT (id = ANY($3)) \
         ORDER BY id LIMIT $2 FOR UPDATE SKIP LOCKED",
    )
    .bind(state.embedder.model_digest())
    .bind(BATCH)
    .bind(&skip)
    .fetch_all(&mut *tx)
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
    let whole = embed_texts(state, texts.clone()).await;
    let results: Vec<Option<Vec<f32>>> = match whole {
        Ok(v) if v.len() == ids.len() => v.into_iter().map(Some).collect(),
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
                out.push(
                    embed_texts(state, vec![t])
                        .await
                        .ok()
                        .and_then(|mut v| v.pop()),
                );
            }
            out
        }
    };
    let mut batch = Batch {
        embedded: 0,
        failed: 0,
    };
    for (id, vector) in ids.iter().zip(&results) {
        let Some(vector) = vector
            .as_ref()
            .filter(|v| v.len() == lampl_core::EMBEDDING_DIM)
        else {
            tracing::warn!(%id, "row could not be embedded; skipping it");
            if let Ok(mut f) = state.failed.lock() {
                f.insert(*id);
            }
            batch.failed += 1;
            continue;
        };
        sqlx::query(
            "UPDATE memories SET embedding = $2, embedding_model = $3, embedding_digest = $4 WHERE id = $1",
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
    let mut last_retry = Instant::now();
    loop {
        match embed_batch(&state).await {
            Ok(b) if b.embedded + b.failed > 0 => {
                tracing::info!(embedded = b.embedded, failed = b.failed, "embedding sweep");
                continue;
            }
            Ok(_) => {
                if last_retry.elapsed() >= RETRY_FAILED_EVERY {
                    if let Ok(mut f) = state.failed.lock() {
                        f.clear();
                    }
                    last_retry = Instant::now();
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
