// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lampl_embed::EmbedError;
use serde_json::json;

use super::{API_VERSION, AppState};

/// How long a recorded embedder outcome may stand in for a live probe when `/health` is asked.
const HEALTH_TTL: Duration = Duration::from_millis(500);
/// Most failed rows `/health` lists by id; `embed_failed_rows` still counts them all.
const MAX_FAILED_LISTED: usize = 50;

/// Outcome of the most recent embedder call, so `/health` reports what the embedder actually did rather than what was configured.
#[derive(Debug)]
pub struct EmbedStatus {
    ok: AtomicBool,
    last_error: Mutex<Option<String>>,
    checked: Mutex<Option<Instant>>,
}

impl EmbedStatus {
    pub(crate) fn new() -> Self {
        EmbedStatus {
            ok: AtomicBool::new(false),
            last_error: Mutex::new(Some("embedder not probed yet".to_owned())),
            checked: Mutex::new(None),
        }
    }

    pub(crate) fn record<T>(&self, result: &Result<T, EmbedError>) {
        let error = result.as_ref().err().map(ToString::to_string);
        self.ok.store(error.is_none(), Ordering::Relaxed);
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = error;
        }
        if let Ok(mut at) = self.checked.lock() {
            *at = Some(Instant::now());
        }
    }

    /// True when an embedder call finished within `ttl`.
    pub(crate) fn fresh(&self, ttl: Duration) -> bool {
        self.checked
            .lock()
            .ok()
            .and_then(|c| *c)
            .is_some_and(|t| t.elapsed() < ttl)
    }

    pub(crate) fn is_ok(&self) -> bool {
        self.ok.load(Ordering::Relaxed)
    }

    fn error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|e| e.clone())
    }
}

pub(crate) async fn health(State(state): State<Arc<AppState>>) -> Response {
    if !state.embed_status.fresh(HEALTH_TTL) {
        super::worker::probe_bounded(&state).await;
    }
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
    let (failed_rows, failed_list) = failed_listing(&state);
    if failed_rows > 0 {
        reasons.push("some rows fail to embed");
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
        "embed_truncation": state.embed_truncation.get(),
        "embed_digest": digest,
        "store": {
            "host": state.store.pool().connect_options().get_host(),
            "port": state.store.pool().connect_options().get_port(),
            "database": state.store.pool().connect_options().get_database(),
        },
        "embedder_available": embedder_ok,
        "embedder_error": if embedder_ok { None } else { state.embed_status.error() },
        "memory_count": total,
        "embedded_count": embedded,
        "embed_failed_rows": failed_rows,
        "embed_failed": failed_list,
        "embed_retry_every_secs": super::worker::RETRY_FAILED_EVERY.as_secs(),
        "embed_next_retry_at": state.next_retry_at().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "vector_coverage": coverage,
        "min_coverage": state.min_coverage,
    }))
    .into_response()
}

/// The failed-row count and the first rows by id with their last embedder error; ids and errors only, never memory text.
fn failed_listing(state: &AppState) -> (usize, Vec<serde_json::Value>) {
    let Ok(failed) = state.failed.lock() else {
        return (0, Vec::new());
    };
    let mut rows: Vec<_> = failed.iter().collect();
    rows.sort_by_key(|(id, _)| **id);
    let listed = rows
        .into_iter()
        .take(MAX_FAILED_LISTED)
        .map(|(id, f)| {
            json!({
                "id": id,
                "error": f.error,
                "failed_at": f.failed_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            })
        })
        .collect();
    (failed.len(), listed)
}

/// `POST /v1/embed/retry-failed`: puts every failed row back in the queue for the next sweep instead of waiting for the retry interval.
pub(crate) async fn retry_failed(State(state): State<Arc<AppState>>) -> Response {
    let retried = state.retry_failed();
    tracing::info!(retried, "failed rows queued for retry on request");
    Json(json!({
        "retried": retried,
        "next_retry_at": state.next_retry_at().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }))
    .into_response()
}
