// SPDX-License-Identifier: Apache-2.0

mod cmd_bench_embed;

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
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::BenchEmbed(args) => cmd_bench_embed::run(args),
    };
    if let Err(e) = result {
        eprintln!("lampl: {e}");
        std::process::exit(1);
    }
}
