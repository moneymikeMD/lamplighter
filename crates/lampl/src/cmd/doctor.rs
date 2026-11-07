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
            let lines = vec![
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
                format!(
                    "local model cache: not needed in server mode ({})",
                    if cached { "present" } else { "absent" }
                ),
            ];
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
