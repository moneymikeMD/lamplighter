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

/// Embeds one batch of rows that have no vector from the current model; returns how many were written.
pub async fn embed_pending(state: &AppState) -> Result<usize, WorkerError> {
    let mut tx = state.store.pool().begin().await?;
    let rows = sqlx::query(
        "SELECT id, title, body FROM memories \
         WHERE embedding IS NULL OR embedding_digest IS DISTINCT FROM $1 \
         ORDER BY id LIMIT $2 FOR UPDATE SKIP LOCKED",
    )
    .bind(state.embedder.model_digest())
    .bind(BATCH)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let mut ids: Vec<Uuid> = Vec::with_capacity(rows.len());
    let mut texts = Vec::with_capacity(rows.len());
    for row in &rows {
        ids.push(row.try_get("id")?);
        let (title, body): (String, String) = (row.try_get("title")?, row.try_get("body")?);
        texts.push(format!("{title}\n\n{body}"));
    }
    let vectors = embed_texts(state, texts).await?;
    if vectors.len() != ids.len() {
        return Err(EmbedError::Embed(format!(
            "embedder returned {} vectors for {} inputs",
            vectors.len(),
            ids.len()
        ))
        .into());
    }
    for (id, vector) in ids.iter().zip(&vectors) {
        if vector.len() != lampl_core::EMBEDDING_DIM {
            return Err(EmbedError::DimMismatch {
                expected: lampl_core::EMBEDDING_DIM,
                got: vector.len(),
            }
            .into());
        }
        sqlx::query(
            "UPDATE memories SET embedding = $2, embedding_model = $3, embedding_digest = $4 WHERE id = $1",
        )
        .bind(id)
        .bind(HalfVector::from_f32_slice(vector))
        .bind(state.embedder.model_id())
        .bind(state.embedder.model_digest())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(ids.len())
}

/// Embeds a short text so `/health` reflects the embedder even when there is nothing to embed.
pub async fn probe(state: &AppState) {
    let _ = embed_texts(state, vec!["health probe".to_owned()]).await;
}

pub(crate) async fn run(state: Arc<AppState>, interval: Duration) {
    let mut last_probe = Instant::now();
    loop {
        match embed_pending(&state).await {
            Ok(n) if n > 0 => {
                tracing::info!(embedded = n, "embedded pending memories");
                continue;
            }
            Ok(_) => {
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
