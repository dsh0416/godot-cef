//! Rust integration runner. Build and packaging tasks live in scripts/build/.

mod integration;
mod support;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "Godot CEF integration test runner")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build the test-only addon and run headless tests against a complete addon
    Integration(integration::Options),
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Commands::Integration(options) => integration::run(options)?,
    }
    Ok(())
}
