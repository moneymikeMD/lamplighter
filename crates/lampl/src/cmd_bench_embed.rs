// SPDX-License-Identifier: Apache-2.0

//! `lampl bench-embed`: per-text embedding latency (p50/p95 per bucket) and optional cosine against an Ollama endpoint.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use clap::Args as ClapArgs;
use lampl_embed::{
    Embedder, EndpointEmbedder, EndpointKind, LocalConfig, LocalEmbedder, Truncator, cosine,
    prepare,
};

#[derive(ClapArgs)]
pub(crate) struct Args {
    /// JSONL file of {"bucket": "...", "text": "..."} lines
    #[arg(long)]
    texts: PathBuf,
    /// Model cache directory (default: platform cache dir)
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// Download the weights from Hugging Face first (the only path that touches the network)
    #[arg(long)]
    prepare: bool,
    /// Timed passes over the text set
    #[arg(long, default_value_t = 3)]
    passes: usize,
    /// Ollama base URL; if set, also report cosine against its vectors (not timed)
    #[arg(long)]
    ollama_url: Option<String>,
    #[arg(long, default_value = "qwen3-embedding:0.6b")]
    ollama_model: String,
}

struct Item {
    bucket: String,
    text: String,
}

pub(crate) fn run(args: Args) -> Result<(), String> {
    let items = read_items(&args.texts)?;
    let mut cfg = LocalConfig::default();
    if let Some(dir) = args.cache_dir {
        cfg.cache_dir = dir;
    }
    if args.prepare {
        prepare(&cfg).map_err(|e| e.to_string())?;
    }
    let t = Instant::now();
    let embedder = LocalEmbedder::load(&cfg).map_err(|e| e.to_string())?;
    let load_ms = t.elapsed().as_secs_f64() * 1e3;

    let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
    embedder.embed(&texts[..1]).map_err(|e| e.to_string())?;

    let mut samples: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    let mut vectors: Vec<Vec<f32>> = Vec::new();
    for pass in 0..args.passes.max(1) {
        for item in &items {
            let t = Instant::now();
            let v = embedder
                .embed(&[item.text.as_str()])
                .map_err(|e| e.to_string())?;
            samples
                .entry(item.bucket.as_str())
                .or_default()
                .push(t.elapsed().as_secs_f64() * 1e3);
            if pass == 0 {
                vectors.push(v.into_iter().next().unwrap_or_default());
            }
        }
    }

    println!("platform: {} / {}", std::env::consts::OS, arch_label());
    println!(
        "model: {} @ {}",
        embedder.model_id(),
        embedder.model_digest()
    );
    println!(
        "threads: {}",
        std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
    );
    println!("model_load_ms: {load_ms:.0}");
    println!("texts: {}  passes: {}", items.len(), args.passes.max(1));
    for (bucket, mut ms) in samples {
        ms.sort_by(f64::total_cmp);
        println!(
            "{bucket}: n={} p50_ms={:.1} p95_ms={:.1} max_ms={:.1}",
            ms.len(),
            pct(&ms, 0.50),
            pct(&ms, 0.95),
            ms[ms.len() - 1]
        );
    }

    match &args.ollama_url {
        None => println!("ollama_cosine: skipped (no --ollama-url)"),
        Some(url) => {
            let mut ollama =
                EndpointEmbedder::new(EndpointKind::Ollama, url, &args.ollama_model, None);
            if let Some(t) = Truncator::cached(&cfg) {
                ollama = ollama.with_truncator(t.map_err(|e| e.to_string())?);
            }
            match ollama.embed(&texts) {
                Err(e) => println!("ollama_cosine: unavailable ({e})"),
                Ok(theirs) => {
                    let mut cos: Vec<f32> = vectors
                        .iter()
                        .zip(&theirs)
                        .map(|(a, b)| cosine(a, b))
                        .collect();
                    cos.sort_by(f32::total_cmp);
                    let mean = cos.iter().sum::<f32>() / cos.len() as f32;
                    println!(
                        "ollama_cosine: n={} mean={mean:.4} min={:.4} max={:.4}",
                        cos.len(),
                        cos[0],
                        cos[cos.len() - 1]
                    );
                }
            }
        }
    }
    Ok(())
}

fn arch_label() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64 (aarch64)",
        "x86_64" => "x86_64",
        other => other,
    }
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

fn read_items(path: &PathBuf) -> Result<Vec<Item>, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let items: Vec<Item> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value =
                serde_json::from_str(l).map_err(|e| format!("bad JSONL line: {e}"))?;
            let text = v["text"]
                .as_str()
                .ok_or("line missing \"text\"")?
                .to_string();
            let bucket = v["bucket"].as_str().unwrap_or("short").to_string();
            Ok(Item { bucket, text })
        })
        .collect::<Result<_, String>>()?;
    if items.is_empty() {
        return Err("no texts".into());
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_pick_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(pct(&v, 0.50), 51.0);
        assert_eq!(pct(&v, 0.95), 95.0);
    }
}
