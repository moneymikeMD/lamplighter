// SPDX-License-Identifier: Apache-2.0

//! lampl-bench: scores lamplighter's retrieval on LongMemEval (evidence-session recall@k and MRR, no LLM judge).

mod checkpoint;
mod dataset;
mod lamplighter;
mod metrics;
mod report;
mod retriever;
mod subset;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};

use retriever::Retriever;
use subset::Subset;

#[derive(Parser)]
#[command(
    name = "lampl-bench",
    version,
    about = "Public retrieval benchmark for lamplighter"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Download and verify the pinned LongMemEval file, then stop
    Fetch(FetchArgs),
    /// Ingest haystacks into a scratch `lampl serve`, recall each question and score it
    Run(RunArgs),
}

#[derive(Args)]
struct FetchArgs {
    #[arg(long)]
    cache_dir: Option<PathBuf>,
}

#[derive(Args)]
struct RunArgs {
    #[arg(long, value_enum, default_value = "ci")]
    subset: Subset,
    /// Question count, overriding the subset's default
    #[arg(long)]
    limit: Option<usize>,
    /// Sessions kept per question (all evidence plus a seeded sample of the rest); `ci` defaults to 12, `full` keeps every session
    #[arg(long)]
    haystack: Option<usize>,
    #[arg(long, default_value_t = subset::DEFAULT_SEED)]
    seed: u64,
    /// Base URL of a scratch `lampl serve`; the API key is read from the LAMPL_API_KEY environment variable
    #[arg(long, default_value = "http://127.0.0.1:7878")]
    server: String,
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// Use this copy of the dataset file instead of downloading (its hash is not checked)
    #[arg(long)]
    dataset: Option<PathBuf>,
    #[arg(long)]
    json_out: Option<PathBuf>,
    /// Run against a store that already holds memories
    #[arg(long)]
    allow_nonempty: bool,
    /// Measure full-text only when the server's embedder is unavailable
    #[arg(long)]
    allow_full_text: bool,
    /// Append each scored question to this JSONL file and, if it already exists, skip the questions it holds; a resumed run may target a store that already holds memories
    #[arg(long)]
    checkpoint: Option<PathBuf>,
    /// Re-rank questions already ingested by an earlier run (its id is the `lme-<id>-` project prefix) without ingesting or embedding documents; the store may be non-empty
    #[arg(long)]
    reuse_run: Option<String>,
    /// Seconds to wait for background embedding to finish
    #[arg(long, default_value_t = 7200)]
    wait_secs: u64,
}

fn run(a: &RunArgs) -> Result<(), String> {
    let key = std::env::var("LAMPL_API_KEY")
        .ok()
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty())
        .ok_or("LAMPL_API_KEY is not set")?;
    let path = match &a.dataset {
        Some(p) => p.clone(),
        None => dataset::ensure(
            &a.cache_dir
                .clone()
                .unwrap_or_else(dataset::default_cache_dir),
        )?,
    };
    let heads = dataset::heads(&path)?;
    let ids = subset::select(&heads, a.subset, a.limit, a.seed);
    if ids.is_empty() {
        return Err("no questions selected".to_owned());
    }
    let wanted: HashSet<String> = ids.iter().cloned().collect();
    let mut questions = dataset::load(&path, &wanted)?;
    let haystack = a.haystack.or(match a.subset {
        Subset::Ci => Some(subset::CI_HAYSTACK),
        Subset::Full => None,
    });
    if let Some(keep) = haystack {
        for q in &mut questions {
            subset::thin(q, keep, a.seed);
        }
    }

    let mut ckpt = match &a.checkpoint {
        Some(p) => Some(checkpoint::Checkpoint::open(
            p,
            &serde_json::json!({
                "dataset": dataset::REVISION,
                "subset": format!("{:?}", a.subset).to_lowercase(),
                "seed": a.seed,
                "haystack": haystack,
                "ids": ids,
            }),
        )?),
        None => None,
    };
    let resumed = ckpt.as_ref().map_or(0, |c| c.done.len());
    if resumed > 0 {
        eprintln!(
            "resuming: {resumed} of {} questions already scored",
            questions.len()
        );
    }

    let mut arm = lamplighter::Lamplighter::connect(
        lamplighter::Options {
            base: a.server.trim_end_matches('/').to_owned(),
            allow_nonempty: a.allow_nonempty || resumed > 0 || a.reuse_run.is_some(),
            allow_full_text: a.allow_full_text,
            wait: Duration::from_secs(a.wait_secs),
            reuse_run: a.reuse_run.clone(),
        },
        key,
    )?;

    let mut rows: Vec<report::Row> = Vec::new();
    for (i, q) in questions.iter().enumerate() {
        if let Some(prior) = ckpt
            .as_ref()
            .and_then(|c| c.done.iter().find(|r| r.id == q.id))
        {
            rows.push(prior.clone());
            continue;
        }
        let started = std::time::Instant::now();
        let stored = if a.reuse_run.is_some() {
            0
        } else {
            let n = arm.ingest(q)?;
            arm.settle()?;
            n
        };
        let ranked = arm.rank(q)?;
        let row = report::Row {
            id: q.id.clone(),
            kind: q.kind.clone(),
            score: metrics::score(&ranked.sessions, &q.answer_session_ids),
            memories: stored,
            mode: ranked.mode,
            hits: ranked.hits,
            sessions: ranked.sessions.len(),
        };
        if let Some(c) = ckpt.as_mut() {
            c.append(row.clone())?;
        }
        rows.push(row);
        eprintln!(
            "scored {}/{} questions ({stored} memories, {:.0}s)",
            i + 1,
            questions.len(),
            started.elapsed().as_secs_f64()
        );
    }
    let memories = if a.reuse_run.is_some() {
        arm.memory_count()?
    } else {
        rows.iter().map(|r| r.memories).sum()
    };
    let modes: HashSet<String> = rows.iter().map(|r| r.mode.clone()).collect();
    let mut modes: Vec<String> = modes.into_iter().collect();
    modes.sort();
    let out = report::Report {
        arm: arm.name().to_owned(),
        dataset: format!("longmemeval_s_cleaned@{}", &dataset::REVISION[..8]),
        subset: format!("{:?}", a.subset).to_lowercase(),
        seed: a.seed,
        haystack_sessions: haystack,
        questions: rows.len(),
        memories,
        recall_modes: modes,
        embedder: arm.info.clone(),
        rows,
    };
    print!("{}", out.table());
    if let Some(p) = &a.json_out {
        std::fs::write(p, out.json()).map_err(|e| format!("writing {}: {e}", p.display()))?;
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Fetch(a) => {
            dataset::ensure(&a.cache_dir.unwrap_or_else(dataset::default_cache_dir))
                .map(|p| println!("{}", p.display()))
        }
        Command::Run(a) => run(&a),
    };
    if let Err(e) = result {
        eprintln!("lampl-bench: {e}");
        std::process::exit(1);
    }
}
