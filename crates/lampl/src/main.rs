// SPDX-License-Identifier: Apache-2.0

mod client;
mod cmd;
mod cmd_bench_embed;
mod import;
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
    /// Run the HTTP API server
    Serve(lampl::serve::Cmd),
    /// Choose local (embedded) or server mode; the only way a store is created
    Init(local::InitArgs),
    /// Store a memory (server or local store, as configured)
    Store(local::StoreArgs),
    /// Recall memories (server or local store, as configured)
    Recall(local::RecallArgs),
    /// Fetch one memory by id
    Get(cmd::GetArgs),
    /// Link two memories
    Link(cmd::LinkArgs),
    /// List the memories linked to one
    Related(cmd::RelatedArgs),
    /// Check the server, its store and embedder; non-zero on any problem
    Doctor(cmd::DoctorArgs),
    /// Report duplicate, stale and untagged memories (read-only)
    Audit(cmd::AuditArgs),
    /// Export every memory and link, with vectors, as NDJSON
    Export(cmd::ExportArgs),
    /// Import a lamplighter or memorygraph export, or read memorygraph's database
    Import(cmd::ImportArgs),
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::BenchEmbed(args) => cmd_bench_embed::run(args),
        Command::Serve(args) => lampl::serve::run(args),
        Command::Init(args) => local::init(args),
        Command::Store(args) => cmd::store(args),
        Command::Recall(args) => cmd::recall(&args),
        Command::Get(args) => cmd::get(&args),
        Command::Link(args) => cmd::link(&args),
        Command::Related(args) => cmd::related(&args),
        Command::Doctor(args) => cmd::doctor(&args),
        Command::Audit(args) => cmd::audit(&args),
        Command::Export(args) => cmd::export(&args),
        Command::Import(args) => cmd::import(&args),
    };
    if let Err(e) = result {
        eprintln!("lampl: {e}");
        std::process::exit(1);
    }
}
