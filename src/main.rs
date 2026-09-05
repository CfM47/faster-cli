use std::process::ExitCode;

use clap::Parser;
use faster_cli::cli::{self, Cli};

#[tokio::main]
async fn main() -> ExitCode {
    cli::run(Cli::parse()).await.into()
}
