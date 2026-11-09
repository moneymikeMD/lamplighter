// SPDX-License-Identifier: Apache-2.0
// unsafe_code is allowed here: pgrx background-worker entry points and the Postgres FFI are unsafe by construction.

//! Experimental pgrx extension: one background worker loads the Qwen3 embedder once and fills `memories.embedding`
//! (plus model id and digest) for rows that lack a vector, so a plain INSERT yields an embedded row. It does not change the store schema.

use std::ffi::CString;
use std::time::Duration;

use lampl_embed::{Embedder, LocalConfig, LocalEmbedder};
use pgrx::bgworkers::{
    BackgroundWorker, BackgroundWorkerBuilder, BgWorkerStartTime, SignalWakeFlags,
};
use pgrx::guc::{GucContext, GucFlags, GucRegistry, GucSetting};
use pgrx::prelude::*;

pgrx::pg_module_magic!(name, version);

static DATABASE: GucSetting<Option<CString>> =
    GucSetting::<Option<CString>>::new(Some(c"postgres"));
static MODEL_DIR: GucSetting<Option<CString>> =
    GucSetting::<Option<CString>>::new(Some(c"/var/lib/lampl/models"));
static POLL_MS: GucSetting<i32> = GucSetting::<i32>::new(250);
static BATCH: GucSetting<i32> = GucSetting::<i32>::new(8);

#[pg_guard]
pub extern "C-unwind" fn _PG_init() {
    GucRegistry::define_string_guc(
        c"lampl.database",
        c"Database the embedding worker connects to.",
        c"Must contain the lamplighter memories table.",
        &DATABASE,
        GucContext::Postmaster,
        GucFlags::default(),
    );
    GucRegistry::define_string_guc(
        c"lampl.model_dir",
        c"Hugging Face cache directory holding Qwen3-Embedding-0.6B.",
        c"Loaded offline; the worker never downloads.",
        &MODEL_DIR,
        GucContext::Postmaster,
        GucFlags::default(),
    );
    GucRegistry::define_int_guc(
        c"lampl.poll_ms",
        c"Milliseconds between scans for rows without an embedding.",
        c"",
        &POLL_MS,
        10,
        60_000,
        GucContext::Sighup,
        GucFlags::default(),
    );
    GucRegistry::define_int_guc(
        c"lampl.batch_size",
        c"Rows embedded per transaction.",
        c"",
        &BATCH,
        1,
        256,
        GucContext::Sighup,
        GucFlags::default(),
    );
    if unsafe { !pg_sys::process_shared_preload_libraries_in_progress } {
        return;
    }
    BackgroundWorkerBuilder::new("lampl embedder")
        .set_function("lampl_embed_worker_main")
        .set_library("lampl_pg_ext")
        .set_type("lampl embedder")
        .enable_spi_access()
        .set_start_time(BgWorkerStartTime::RecoveryFinished)
        .set_restart_time(Some(Duration::from_secs(5)))
        .load();
}

#[pg_guard]
#[unsafe(no_mangle)]
pub extern "C-unwind" fn lampl_embed_worker_main(_arg: pg_sys::Datum) {
    BackgroundWorker::attach_signal_handlers(SignalWakeFlags::SIGHUP | SignalWakeFlags::SIGTERM);
    let database = DATABASE
        .get()
        .and_then(|c| c.into_string().ok())
        .unwrap_or_else(|| "postgres".to_string());
    BackgroundWorker::connect_worker_to_spi(Some(&database), None);

    let cache_dir = MODEL_DIR
        .get()
        .and_then(|c| c.into_string().ok())
        .unwrap_or_default();
    let started = std::time::Instant::now();
    let embedder = match LocalEmbedder::load(&LocalConfig {
        cache_dir: cache_dir.into(),
        ..LocalConfig::default()
    }) {
        Ok(e) => e,
        Err(e) => error!("lampl embedder: {e}"),
    };
    log!(
        "lampl embedder: model loaded in {} ms",
        started.elapsed().as_millis()
    );

    while BackgroundWorker::wait_latch(Some(Duration::from_millis(POLL_MS.get() as u64))) {
        if BackgroundWorker::sighup_received() {
            log!("lampl embedder: configuration reloaded");
        }
        loop {
            let embedder = std::panic::AssertUnwindSafe(&embedder);
            let done = BackgroundWorker::transaction(|| embed_pending(&embedder));
            if done == 0 {
                break;
            }
        }
    }
}

fn schema_ready() -> bool {
    Spi::get_one::<bool>(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'memories' AND column_name = 'embedding_digest')",
    )
    .ok()
    .flatten()
    .unwrap_or(false)
}

fn embed_pending(embedder: &LocalEmbedder) -> usize {
    if !schema_ready() {
        return 0;
    }
    let limit = i64::from(BATCH.get());
    let rows: Vec<(String, String)> = Spi::connect(|client| {
        let table = client.select(
            "SELECT id::text, title || E'\\n\\n' || body FROM memories WHERE embedding IS NULL \
             ORDER BY created_at LIMIT $1 FOR UPDATE SKIP LOCKED",
            None,
            &[limit.into()],
        )?;
        let mut out = Vec::new();
        for row in table {
            let id = row.get::<String>(1)?.unwrap_or_default();
            let text = row.get::<String>(2)?.unwrap_or_default();
            out.push((id, text));
        }
        Ok::<_, pgrx::spi::Error>(out)
    })
    .unwrap_or_else(|e| error!("lampl embedder: select failed: {e}"));
    if rows.is_empty() {
        return 0;
    }
    let texts: Vec<&str> = rows.iter().map(|(_, t)| t.as_str()).collect();
    let vectors = embedder
        .embed(&texts)
        .unwrap_or_else(|e| error!("lampl embedder: {e}"));
    for ((id, _), vector) in rows.iter().zip(vectors) {
        let literal = format!(
            "[{}]",
            vector
                .iter()
                .map(f32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        );
        Spi::run_with_args(
            "UPDATE memories SET embedding = $1::halfvec, embedding_model = $2, embedding_digest = $3 WHERE id = $4::uuid",
            &[
                literal.into(),
                embedder.model_id().to_string().into(),
                embedder.model_digest().to_string().into(),
                id.clone().into(),
            ],
        )
        .unwrap_or_else(|e| error!("lampl embedder: update failed: {e}"));
    }
    rows.len()
}

#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {}

    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec![]
    }
}
