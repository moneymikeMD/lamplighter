// SPDX-License-Identifier: Apache-2.0

//! `lampl serve`: the versioned HTTP API that owns migrations, embedding, ranking and API-key auth.

mod auth;
mod error;
mod handlers;
mod health;
mod worker;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::HeaderValue;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use lampl_embed::{
    EmbedError, Embedder, EndpointEmbedder, EndpointKind, LocalConfig, LocalEmbedder,
};
use lampl_store_pg::PgStore;

pub use auth::{KeyError, KeyStore, generate_key, hash_key};
pub use health::EmbedStatus;
pub use worker::{WorkerError, embed_pending, probe};

use error::ApiError;

/// The explicit API compatibility integer; bump it on any breaking change to `/v1` semantics, independent of the crate semver.
pub const API_VERSION: u32 = 1;
pub const API_VERSION_HEADER: &str = "x-lampl-api-version";

const IMPORT_LIMIT: usize = 256 * 1024 * 1024;

pub type SharedEmbedder = Arc<dyn Embedder + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedMode {
    InProcess,
    Endpoint,
}

impl EmbedMode {
    pub fn as_str(self) -> &'static str {
        match self {
            EmbedMode::InProcess => "in-process",
            EmbedMode::Endpoint => "endpoint",
        }
    }
}

pub struct AppState {
    pub(crate) store: PgStore,
    pub(crate) embedder: SharedEmbedder,
    pub(crate) embed_mode: EmbedMode,
    pub(crate) keys: KeyStore,
    pub(crate) embed_status: EmbedStatus,
    pub(crate) min_coverage: f64,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("embed_mode", &self.embed_mode)
            .field("min_coverage", &self.min_coverage)
            .finish_non_exhaustive()
    }
}

impl AppState {
    pub fn new(
        store: PgStore,
        embedder: SharedEmbedder,
        embed_mode: EmbedMode,
        keys: KeyStore,
        min_coverage: f64,
    ) -> Arc<Self> {
        Arc::new(AppState {
            store,
            embedder,
            embed_mode,
            keys,
            embed_status: EmbedStatus::new(),
            min_coverage,
        })
    }
}

async fn version_header(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    resp.headers_mut()
        .insert(API_VERSION_HEADER, HeaderValue::from(API_VERSION));
    resp
}

async fn fallback() -> Response {
    ApiError::not_found().into_response()
}

pub fn router(state: Arc<AppState>) -> Router {
    use handlers::{
        create_link, export, get_memory, import, list_memories, recall, related, store_memory,
    };
    let v1 = Router::new()
        .route("/memories", post(store_memory).get(list_memories))
        .route("/memories/{id}", get(get_memory))
        .route("/memories/{id}/related", get(related))
        .route("/recall", post(recall))
        .route("/links", post(create_link))
        .route("/export", get(export))
        .route(
            "/import",
            post(import).layer(DefaultBodyLimit::max(IMPORT_LIMIT)),
        )
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth::require_key,
        ));
    Router::new()
        .route("/health", get(health::health))
        .nest("/v1", v1)
        .fallback(fallback)
        .layer(middleware::from_fn(version_header))
        .with_state(state)
}

/// Stands in for an embedder that could not be built, so the server starts and `/health` says so.
struct UnavailableEmbedder {
    reason: String,
}

impl Embedder for UnavailableEmbedder {
    fn embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Err(EmbedError::Load(self.reason.clone()))
    }
    fn dim(&self) -> usize {
        lampl_embed::DIM
    }
    fn model_id(&self) -> &str {
        lampl_embed::DEFAULT_REPO_ID
    }
    fn model_digest(&self) -> &str {
        "unavailable"
    }
}

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Address to listen on
    #[arg(long, default_value = "127.0.0.1:7878")]
    listen: SocketAddr,
    /// Postgres URL (default: $LAMPL_DATABASE_URL; prefer the environment so the password stays out of argv)
    #[arg(long)]
    database_url: Option<String>,
    /// File of `<client> <sha256-hex>` lines (default: $LAMPL_KEYS_FILE); create entries with `lampl keygen`
    #[arg(long)]
    keys_file: Option<PathBuf>,
    /// Use an HTTP embedding endpoint instead of the in-process model
    #[arg(long)]
    embed_endpoint: Option<String>,
    /// Endpoint protocol: openai or ollama
    #[arg(long, default_value = "openai")]
    embed_endpoint_kind: String,
    /// Model name sent to the endpoint
    #[arg(long, default_value = lampl_embed::DEFAULT_REPO_ID)]
    embed_model: String,
    /// /health reports degraded below this vector coverage (0..=1)
    #[arg(long, default_value_t = 0.95)]
    min_coverage: f64,
    /// Seconds between background embedding sweeps
    #[arg(long, default_value_t = 5)]
    embed_interval_secs: u64,
}

#[derive(clap::Args, Debug)]
pub struct KeygenArgs {
    /// Client name, e.g. the machine this key is for
    name: String,
}

pub fn keygen(args: &KeygenArgs) -> Result<(), String> {
    if args.name.is_empty() || args.name.chars().any(char::is_whitespace) {
        return Err("client name must be non-empty and contain no whitespace".to_owned());
    }
    let key = generate_key().map_err(|e| e.to_string())?;
    println!("key (give to the client, shown once): {key}");
    println!(
        "keys-file line (append on the server): {} {}",
        args.name,
        hash_key(&key)
    );
    Ok(())
}

fn build_embedder(args: &Args) -> Result<(SharedEmbedder, EmbedMode), String> {
    if let Some(url) = &args.embed_endpoint {
        let kind = match args.embed_endpoint_kind.as_str() {
            "openai" => EndpointKind::OpenAi,
            "ollama" => EndpointKind::Ollama,
            other => {
                return Err(format!(
                    "unknown --embed-endpoint-kind `{other}`; expected openai or ollama"
                ));
            }
        };
        let key = std::env::var("LAMPL_EMBED_API_KEY").ok();
        let e = EndpointEmbedder::new(kind, url, &args.embed_model, key);
        return Ok((Arc::new(e), EmbedMode::Endpoint));
    }
    let embedder: SharedEmbedder = match LocalEmbedder::load(&LocalConfig::default()) {
        Ok(e) => Arc::new(e),
        Err(e) => {
            tracing::error!(error = %e, "in-process embedder unavailable; serving full-text only");
            Arc::new(UnavailableEmbedder {
                reason: e.to_string(),
            })
        }
    };
    Ok((embedder, EmbedMode::InProcess))
}

pub fn run(args: Args) -> Result<(), String> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .try_init()
        .ok();
    if !(0.0..=1.0).contains(&args.min_coverage) {
        return Err("--min-coverage must be between 0 and 1".to_owned());
    }
    let database_url = args
        .database_url
        .clone()
        .or_else(|| std::env::var("LAMPL_DATABASE_URL").ok())
        .ok_or("no database URL: set LAMPL_DATABASE_URL or pass --database-url")?;
    let keys_path = args
        .keys_file
        .clone()
        .or_else(|| std::env::var_os("LAMPL_KEYS_FILE").map(PathBuf::from))
        .ok_or("no keys file: set LAMPL_KEYS_FILE or pass --keys-file (refusing to serve unauthenticated)")?;
    let keys = KeyStore::open(&keys_path).map_err(|e| e.to_string())?;
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    rt.block_on(async move {
        let store = PgStore::connect(&database_url).await.map_err(|e| format!("database: {e}"))?;
        let found = store.schema_version().await.map_err(|e| e.to_string())?;
        let known = PgStore::supported_schema_version();
        if found > known {
            return Err(format!(
                "database schema version {found} is newer than this build understands ({known}); upgrade lampl"
            ));
        }
        let (embedder, mode) = {
            let args = &args;
            tokio::task::block_in_place(|| build_embedder(args))?
        };
        let state = AppState::new(store, embedder, mode, keys, args.min_coverage);
        probe(&state).await;
        tokio::spawn(worker::run(
            Arc::clone(&state),
            Duration::from_secs(args.embed_interval_secs.max(1)),
        ));
        let listener = tokio::net::TcpListener::bind(args.listen)
            .await
            .map_err(|e| format!("bind {}: {e}", args.listen))?;
        tracing::info!(listen = %args.listen, schema = found, api_version = API_VERSION, embed_mode = mode.as_str(), "lampl serve ready");
        axum::serve(listener, router(state))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .map_err(|e| e.to_string())
    })
}
