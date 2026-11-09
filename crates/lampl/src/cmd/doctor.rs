// SPDX-License-Identifier: Apache-2.0

use clap::Args as ClapArgs;
use lampl_embed::{LocalConfig, model_cached, prepare};
use serde_json::{Value, json};

use crate::client::{Client, Target, target};
use lampl::serve::API_VERSION;

#[derive(ClapArgs)]
pub(crate) struct DoctorArgs {
    /// Machine-readable report
    #[arg(long)]
    json: bool,
    /// Download the embedding model into the local cache (the only place a download happens)
    #[arg(long)]
    fetch_model: bool,
    /// Ask the server to retry rows that failed to embed now instead of at the next scheduled retry
    #[arg(long)]
    retry_failed: bool,
}

/// Rows listed in plain output; `--json` carries the server's full list.
const FAILED_SHOWN: usize = 10;

fn failed_lines(h: &Value) -> Vec<String> {
    let count = h["embed_failed_rows"].as_u64().unwrap_or(0);
    if count == 0 {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "rows failing to embed: {count} (next retry due {}; `lampl doctor --retry-failed` retries now)",
        h["embed_next_retry_at"].as_str().unwrap_or("?")
    )];
    let rows = h["embed_failed"].as_array().map_or(&[][..], Vec::as_slice);
    for r in rows.iter().take(FAILED_SHOWN) {
        lines.push(format!(
            "  {}: {}",
            r["id"].as_str().unwrap_or("?"),
            r["error"].as_str().unwrap_or("?")
        ));
    }
    let shown = u64::try_from(rows.len().min(FAILED_SHOWN)).unwrap_or(u64::MAX);
    if count > shown {
        lines.push(format!(
            "  … and {} more (`lampl doctor --json`)",
            count - shown
        ));
    }
    lines
}

fn emit(json_out: bool, report: &Value, lines: &[String]) {
    if json_out {
        println!("{report}");
    } else {
        for l in lines {
            println!("{l}");
        }
    }
}

pub(crate) fn doctor(args: &DoctorArgs) -> Result<(), String> {
    let cfg = LocalConfig::default();
    if args.fetch_model {
        prepare(&cfg).map_err(|e| e.to_string())?;
        eprintln!("embedding model downloaded to {}", cfg.cache_dir.display());
    }
    let cached = model_cached(&cfg);
    match target()? {
        Target::Local if args.retry_failed => Err(
            "`lampl doctor --retry-failed` needs a server; local mode embeds on store".to_owned(),
        ),
        Target::Local => {
            let ok = cached;
            let report = json!({"mode": "local", "model_cache": cfg.cache_dir, "model_cached": cached, "ok": ok});
            emit(
                args.json,
                &report,
                &[
                    "mode: local".to_owned(),
                    format!(
                        "model cache: {} ({})",
                        cfg.cache_dir.display(),
                        if cached { "present" } else { "MISSING" }
                    ),
                ],
            );
            if ok {
                Ok(())
            } else {
                Err("model cache missing; run `lampl doctor --fetch-model`".to_owned())
            }
        }
        Target::Server(settings) => {
            let client = Client::connect(&settings)?;
            if args.retry_failed {
                let r = client.post("/v1/embed/retry-failed", &json!({}))?;
                eprintln!(
                    "queued {} failed row(s) for the next embedding sweep",
                    r["retried"]
                );
            }
            let h = client.get("/health")?;
            let mut problems: Vec<String> = h["degraded_reasons"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| r.as_str().map(str::to_owned))
                .collect();
            if h["degraded"] == true && problems.is_empty() {
                problems.push("server reports degraded".to_owned());
            }
            let mut lines = vec![
                format!("server: {}", client.base),
                format!("api version: {API_VERSION} (server agrees)"),
                format!(
                    "store: {}:{}/{}",
                    h["store"]["host"].as_str().unwrap_or("?"),
                    h["store"]["port"],
                    h["store"]["database"].as_str().unwrap_or("?")
                ),
                format!("schema version: {}", h["schema_version"]),
                format!(
                    "memories: {} ({} embedded)",
                    h["memory_count"], h["embedded_count"]
                ),
                format!(
                    "embed: {} / {}",
                    h["embed_mode"].as_str().unwrap_or("?"),
                    h["embed_model"].as_str().unwrap_or("?")
                ),
                format!(
                    "vector coverage: {} (minimum {})",
                    h["vector_coverage"], h["min_coverage"]
                ),
                format!("degraded: {}", h["degraded"]),
            ];
            if let Some(t) = h["embed_truncation"].as_str() {
                lines.push(if t.starts_with("chars:") {
                    format!("embed input cut: {t} (approximate; run `lampl serve --fetch-tokenizer` on the server host for the exact 512-token cut)")
                } else {
                    format!("embed input cut: {t}")
                });
            }
            lines.extend(failed_lines(&h));
            lines.push(format!(
                "local model cache: not needed in server mode ({})",
                if cached { "present" } else { "absent" }
            ));
            let mut report = json!({"server": client.base, "api_version": API_VERSION, "health": h, "ok": problems.is_empty()});
            report["problems"] = json!(problems);
            emit(args.json, &report, &lines);
            if problems.is_empty() {
                Ok(())
            } else {
                Err(format!("doctor found problems: {}", problems.join("; ")))
            }
        }
    }
}
