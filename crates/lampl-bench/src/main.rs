// SPDX-License-Identifier: Apache-2.0

//! lampl-bench: scores lamplighter's retrieval on LongMemEval (evidence-session recall@k and MRR, no LLM judge).

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

    let mut arm = lamplighter::Lamplighter::connect(
        lamplighter::Options {
            base: a.server.trim_end_matches('/').to_owned(),
            allow_nonempty: a.allow_nonempty,
            allow_full_text: a.allow_full_text,
            wait: Duration::from_secs(a.wait_secs),
        },
        key,
    )?;

    let mut memories = 0;
    for (i, q) in questions.iter().enumerate() {
        memories += arm.ingest(q)?;
        eprintln!(
            "ingested {}/{} questions ({memories} memories)",
            i + 1,
            questions.len()
        );
    }
    arm.settle()?;

    let mut rows = Vec::new();
    let mut modes = HashSet::new();
    for q in &questions {
        let ranked = arm.rank(q)?;
        modes.insert(ranked.mode.clone());
        rows.push(report::Row {
            id: q.id.clone(),
            kind: q.kind.clone(),
            score: metrics::score(&ranked.sessions, &q.answer_session_ids),
        });
    }
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
