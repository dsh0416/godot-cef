//! Validation command - checks packaged addon layout and required artifacts

use crate::bundle_common::validate_required_paths;
use crate::platform::PLATFORM_SPECS;
use std::path::Path;

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
    for platform in PLATFORM_SPECS {
        let platform_dir = bin_dir.join(platform.target);
        if !platform_dir.exists() {
            println!("Skipping {} (not present)", platform.target);
            continue;
        }

        validate_required_paths(
            &platform_dir,
            platform.required_files,
            platform.required_dirs,
        )?;
        println!("Validated {}", platform.target);
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
