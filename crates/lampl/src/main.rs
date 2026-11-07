// SPDX-License-Identifier: Apache-2.0

mod cmd_bench_embed;
mod local;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "lampl", version, about = "Agent memory as one binary")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Measure embedding latency on this machine
    BenchEmbed(cmd_bench_embed::Args),
    /// Choose local (embedded) or server mode; the only way a store is created
    Init(local::InitArgs),
    /// Store a memory in the configured local store
    Store(local::StoreArgs),
    /// Recall memories from the configured local store
    Recall(local::RecallArgs),
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::BenchEmbed(args) => cmd_bench_embed::run(args),
        Command::Init(args) => local::init(args),
        Command::Store(args) => local::store(args),
        Command::Recall(args) => local::recall(args),
    };
    if let Err(e) = result {
        eprintln!("lampl: {e}");
        std::process::exit(1);
    }
}
