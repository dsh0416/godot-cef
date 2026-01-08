//! xtask - Build tasks for gdcef
//!
//! Usage:
//!   cargo xtask bundle [--release]           # Bundle for the current platform
//!   cargo xtask bundle-app [--release]       # Bundle helper app (macOS only)
//!   cargo xtask bundle-framework [--release] # Bundle framework (macOS only)

#[cfg(target_os = "macos")]
mod bundle_app;
mod bundle_common;
#[cfg(target_os = "macos")]
mod bundle_framework;
#[cfg(target_os = "linux")]
mod bundle_linux;
#[cfg(target_os = "windows")]
mod bundle_windows;

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
    /// Bundle for the current platform (cross-platform)
    Bundle {
        /// Build in release mode
        #[arg(long, short)]
        release: bool,

        /// Custom target directory
        #[arg(long)]
        target_dir: Option<PathBuf>,
    },

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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Bundle {
            release,
            target_dir,
        } => {
            #[cfg(target_os = "macos")]
            {
                bundle_app::run(release, target_dir.as_deref())?;
                bundle_framework::run(release, target_dir.as_deref())?;
            }

            #[cfg(target_os = "windows")]
            {
                bundle_windows::run(release, target_dir.as_deref())?;
            }

            #[cfg(target_os = "linux")]
            {
                bundle_linux::run(release, target_dir.as_deref())?;
            }
        }
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
    }

    Ok(())
}
