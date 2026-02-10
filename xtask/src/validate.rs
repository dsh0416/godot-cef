//! Validation command - checks packaged addon layout and required artifacts

use crate::bundle_common::{required_paths_for_platform, validate_required_paths};
use std::path::Path;

const PLATFORM_TARGETS: &[&str] = &[
    "universal-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
];

pub fn run(addon_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let bin_dir = addon_dir.join("bin");
    if !bin_dir.exists() {
        return Err(format!(
            "Addon directory '{}' does not contain a bin/ directory",
            addon_dir.display()
        )
        .into());
    }

    let mut validated = 0usize;
    for platform in PLATFORM_TARGETS {
        let platform_dir = bin_dir.join(platform);
        if !platform_dir.exists() {
            println!("Skipping {} (not present)", platform);
            continue;
        }

        let (files, dirs) = required_paths_for_platform(platform);
        validate_required_paths(&platform_dir, files, dirs)?;
        println!("Validated {}", platform);
        validated += 1;
    }

    if validated == 0 {
        return Err("No platform directories found under addon bin/".into());
    }

    println!(
        "Validation complete: {} platform(s) checked in {}",
        validated,
        addon_dir.display()
    );
    Ok(())
}
