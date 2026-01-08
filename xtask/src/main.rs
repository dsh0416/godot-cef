//! xtask - Build tasks for gdcef
//!
//! Usage:
//!   cargo xtask bundle-app [--release]
//!   cargo xtask bundle-framework [--release]
//!   cargo xtask bundle-all [--release]

mod bundle_app;
mod bundle_common;
mod bundle_framework;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "Build tasks for gdcef", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Bundle the helper app for macOS
    BundleApp {
        /// Build in release mode
        #[arg(long, short)]
        release: bool,

        /// Custom target directory
        #[arg(long)]
        target_dir: Option<PathBuf>,
    },

    /// Bundle the GDExtension framework for macOS
    BundleFramework {
        /// Build in release mode
        #[arg(long, short)]
        release: bool,

        /// Custom target directory
        #[arg(long)]
        target_dir: Option<PathBuf>,
    },

    /// Bundle both the helper app and framework for macOS
    BundleAll {
        /// Build in release mode
        #[arg(long, short)]
        release: bool,

        /// Custom target directory
        #[arg(long)]
        target_dir: Option<PathBuf>,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!(
            "Warning: Bundling is only supported on macOS. Commands will be no-ops on other platforms."
        );
    }

    match cli.command {
        Commands::BundleApp {
            release,
            target_dir,
        } => {
            #[cfg(target_os = "macos")]
            bundle_app::run(release, target_dir.as_deref())?;

            #[cfg(not(target_os = "macos"))]
            {
                let _ = (release, target_dir);
                eprintln!("bundle-app is only supported on macOS");
            }
        }
        Commands::BundleFramework {
            release,
            target_dir,
        } => {
            #[cfg(target_os = "macos")]
            bundle_framework::run(release, target_dir.as_deref())?;

            #[cfg(not(target_os = "macos"))]
            {
                let _ = (release, target_dir);
                eprintln!("bundle-framework is only supported on macOS");
            }
        }
        Commands::BundleAll {
            release,
            target_dir,
        } => {
            #[cfg(target_os = "macos")]
            {
                bundle_app::run(release, target_dir.as_deref())?;
                bundle_framework::run(release, target_dir.as_deref())?;
            }

            #[cfg(not(target_os = "macos"))]
            {
                let _ = (release, target_dir);
                eprintln!("bundle-all is only supported on macOS");
            }
        }
    }

    Ok(())
}
