// SPDX-License-Identifier: Apache-2.0

//! Embedded local mode. A store is created only by `lampl init --local`; a client configured for a server never falls back to it.

pub(crate) mod config;

use std::path::Path;

use clap::Args as ClapArgs;
use lampl_core::{Embedding, MemoryType, NewMemory, query_embed_text};
use lampl_embed::{Embedder, LocalConfig, LocalEmbedder, prepare};
use lampl_store_sqlite::{RecallQuery, SqliteStore};

use config::Config;

#[derive(ClapArgs)]
pub(crate) struct InitArgs {
    /// Create an embedded SQLite store on this machine and record `mode = local`
    #[arg(long, conflicts_with = "server")]
    local: bool,
    /// Record `mode = server` for the given base URL; repeat for fallback URLs tried in order
    #[arg(long, value_name = "URL")]
    server: Vec<String>,
    /// With --server: file holding the API key (otherwise $LAMPL_API_KEY is used)
    #[arg(long, value_name = "PATH", requires = "server")]
    api_key_file: Option<std::path::PathBuf>,
    /// With --local: download the embedding model now (the only network step)
    #[arg(long, requires = "local")]
    prepare: bool,
}

#[derive(ClapArgs)]
pub(crate) struct StoreArgs {
    #[arg(long)]
    pub(crate) title: String,
    #[arg(long)]
    pub(crate) body: String,
    #[arg(long = "type", default_value = "fact")]
    pub(crate) memory_type: String,
    #[arg(long, value_delimiter = ',')]
    pub(crate) tags: Vec<String>,
    #[arg(long, default_value = "default")]
    pub(crate) project: String,
    #[arg(long, default_value_t = 0.5)]
    pub(crate) importance: f32,
    /// Skip embedding; the memory is found by full text only
    #[arg(long)]
    pub(crate) no_embed: bool,
}

#[derive(ClapArgs)]
pub(crate) struct RecallArgs {
    #[arg(required_unless_present = "query_flag", conflicts_with = "query_flag")]
    query: Option<String>,
    /// The query, as a flag
    #[arg(long = "query", value_name = "TEXT")]
    query_flag: Option<String>,
    #[arg(long)]
    pub(crate) project: Option<String>,
    #[arg(long, default_value_t = 10)]
    pub(crate) limit: usize,
    /// Skip embedding; recall by full text only
    #[arg(long)]
    pub(crate) no_embed: bool,
    /// Print one `{"hits": [...], "warning": ...}` object; an empty recall is `{"hits": []}`
    #[arg(long)]
    pub(crate) json: bool,
    /// Give up embedding the query after this many ms and recall by full text; env `LAMPL_EMBED_TIMEOUT_MS`
    #[arg(long, value_name = "MS")]
    pub(crate) embed_timeout_ms: Option<u64>,
}

impl RecallArgs {
    pub(crate) fn new(query: String, project: Option<String>, limit: usize) -> Self {
        Self {
            query: Some(query),
            query_flag: None,
            project,
            limit,
            no_embed: false,
            json: false,
            embed_timeout_ms: None,
        }
    }

    pub(crate) fn embed_timeout(&self) -> Option<std::time::Duration> {
        self.embed_timeout_ms
            .or_else(|| std::env::var("LAMPL_EMBED_TIMEOUT_MS").ok()?.parse().ok())
            .map(std::time::Duration::from_millis)
    }

    pub(crate) fn text(&self) -> &str {
        self.query
            .as_deref()
            .or(self.query_flag.as_deref())
            .unwrap_or_default()
    }
}

pub(crate) fn init(args: InitArgs) -> Result<(), String> {
    let path = config::config_path()?;
    let existing = match config::load() {
        Ok(c) => Some(c),
        Err(e) if e.starts_with("not configured") => None,
        Err(e) => return Err(e),
    };
    if let Some((url, rest)) = args.server.split_first() {
        if matches!(existing, Some(Config::Local { .. })) {
            return Err(format!(
                "{} is already configured for local mode; remove it to switch",
                path.display()
            ));
        }
        config::save(
            &path,
            &Config::Server {
                url: url.trim_end_matches('/').to_owned(),
                fallback_urls: rest
                    .iter()
                    .map(|u| u.trim_end_matches('/').to_owned())
                    .collect(),
                api_key_file: args.api_key_file,
            },
        )?;
        println!("mode = server ({url}); config at {}", path.display());
        return Ok(());
    }
    if !args.local {
        return Err("choose a mode: `lampl init --local` or `lampl init --server <URL>`".into());
    }
    if matches!(existing, Some(Config::Server { .. })) {
        return Err(format!(
            "{} is already configured for a server; remove it to switch to local mode",
            path.display()
        ));
    }
    let db_path = match existing {
        Some(Config::Local { db_path }) => db_path,
        _ => config::data_dir()?.join(config::DB_FILE),
    };
    if let Some(dir) = db_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    SqliteStore::open(&db_path).map_err(|e| e.to_string())?;
    config::save(
        &path,
        &Config::Local {
            db_path: db_path.clone(),
        },
    )?;
    println!(
        "mode = local; database {}; config at {}",
        db_path.display(),
        path.display()
    );
    if args.prepare {
        prepare(&LocalConfig::default()).map_err(|e| e.to_string())?;
        println!("embedding model downloaded");
    } else {
        println!(
            "embedding model not downloaded; run `lampl init --local --prepare` once, or pass --no-embed"
        );
    }
    Ok(())
}

fn open_local() -> Result<SqliteStore, String> {
    match config::load()? {
        Config::Local { db_path } => open_existing(&db_path),
        Config::Server { .. } => Err("configured for a server, not local mode".to_owned()),
    }
}

fn open_existing(db_path: &Path) -> Result<SqliteStore, String> {
    if !db_path.exists() {
        return Err(format!(
            "local database {} is missing; run `lampl init --local` to create a new one",
            db_path.display()
        ));
    }
    SqliteStore::open(db_path).map_err(|e| e.to_string())
}

fn embed(text: &str) -> Result<Embedding, String> {
    let embedder = LocalEmbedder::load(&LocalConfig::default()).map_err(|e| e.to_string())?;
    let vector = embedder
        .embed(&[text])
        .map_err(|e| e.to_string())?
        .into_iter()
        .next()
        .ok_or("embedder returned no vector")?;
    Ok(Embedding {
        vector,
        model_id: embedder.model_id().to_owned(),
        model_digest: embedder.model_digest().to_owned(),
    })
}

pub(crate) fn store_value(args: StoreArgs) -> Result<serde_json::Value, String> {
    let mut db = open_local()?;
    let memory_type: MemoryType = args.memory_type.parse().map_err(|e| format!("{e}"))?;
    let embedding = if args.no_embed {
        None
    } else {
        Some(embed(&format!("{}\n{}", args.title, args.body))?)
    };
    let memory = db
        .store(NewMemory {
            memory_type,
            title: args.title,
            body: args.body,
            tags: args.tags,
            project: args.project,
            importance: args.importance,
            embedding,
        })
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&memory).map_err(|e| e.to_string())
}

pub(crate) const EMBED_TIMED_OUT: &str =
    "embedding the query timed out; this recall is full-text only";

pub(crate) fn recall_hits(
    args: &RecallArgs,
) -> Result<(Vec<serde_json::Value>, Option<&'static str>), String> {
    let db = open_local()?;
    let mut q = RecallQuery::new(args.text());
    q.project.clone_from(&args.project);
    q.limit = args.limit;
    let mut warning = None;
    if !args.no_embed {
        match args.embed_timeout() {
            None => q.embedding = Some(embed(&query_embed_text(args.text()))?.vector),
            Some(limit) => {
                let (tx, rx) = std::sync::mpsc::channel();
                let text = query_embed_text(args.text());
                std::thread::spawn(move || {
                    let _ = tx.send(embed(&text).map(|e| e.vector));
                });
                match rx.recv_timeout(limit) {
                    Ok(vector) => q.embedding = Some(vector?),
                    Err(_) => warning = Some(EMBED_TIMED_OUT),
                }
            }
        }
    }
    let hits = db
        .recall(&q)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|hit| {
            serde_json::json!({
                "score": hit.score,
                "cosine": hit.cosine,
                "coverage": hit.coverage,
                "memory": hit.memory,
            })
        })
        .collect();
    Ok((hits, warning))
}
