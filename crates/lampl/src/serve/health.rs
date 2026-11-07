// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lampl_embed::EmbedError;
use serde_json::json;

use super::{API_VERSION, AppState};

/// Outcome of the most recent embedder call, so `/health` reports what the embedder actually did rather than what was configured.
#[derive(Debug)]
pub struct EmbedStatus {
    ok: AtomicBool,
    last_error: Mutex<Option<String>>,
}

impl EmbedStatus {
    pub(crate) fn new() -> Self {
        EmbedStatus {
            ok: AtomicBool::new(false),
            last_error: Mutex::new(Some("embedder not probed yet".to_owned())),
        }
    }

    pub(crate) fn record<T>(&self, result: &Result<T, EmbedError>) {
        let error = result.as_ref().err().map(ToString::to_string);
        self.ok.store(error.is_none(), Ordering::Relaxed);
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = error;
        }
    }

    pub(crate) fn is_ok(&self) -> bool {
        self.ok.load(Ordering::Relaxed)
    }

    fn error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|e| e.clone())
    }
}

pub(crate) async fn health(State(state): State<Arc<AppState>>) -> Response {
    let digest = state.embedder.model_digest().to_owned();
    let counts: Result<(i64, i64), sqlx::Error> = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE embedding IS NOT NULL AND embedding_digest = $1) FROM memories",
    )
    .bind(&digest)
    .fetch_one(state.store.pool())
    .await;
    let schema = state.store.schema_version().await;
    let (Ok((total, embedded)), Ok(schema_version)) = (counts, schema) else {
        tracing::error!("health: database unreachable");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "unavailable",
                "degraded": true,
                "api_version": API_VERSION,
                "error": "database unreachable",
            })),
        )
            .into_response();
    };
    let coverage = if total == 0 {
        1.0
    } else {
        embedded as f64 / total as f64
    };
    let embedder_ok = state.embed_status.is_ok();
    let mut reasons = Vec::new();
    if !embedder_ok {
        reasons.push("embedder unavailable: recall is full-text only");
    }
    if coverage < state.min_coverage {
        reasons.push("vector coverage below threshold");
    }
    let degraded = !reasons.is_empty();
    Json(json!({
        "status": if degraded { "degraded" } else { "ok" },
        "degraded": degraded,
        "degraded_reasons": reasons,
        "api_version": API_VERSION,
        "schema_version": schema_version,
        "embed_mode": state.embed_mode.as_str(),
        "embed_model": state.embedder.model_id(),
        "embedder_available": embedder_ok,
        "embedder_error": if embedder_ok { None } else { state.embed_status.error() },
        "memory_count": total,
        "embedded_count": embedded,
        "vector_coverage": coverage,
        "min_coverage": state.min_coverage,
    }))
    .into_response()
}
