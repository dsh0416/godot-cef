use crate::bundle_common::workspace_root;
use std::fs;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let root = workspace_root();
    let cargo_toml = fs::read_to_string(root.join("Cargo.toml"))?;
    let cargo_lock = fs::read_to_string(root.join("Cargo.lock"))?;
    let mise_toml = fs::read_to_string(root.join("mise.toml"))?;
    let package_json = fs::read_to_string(root.join("package.json"))?;

    let workspace_version = quoted_value(&cargo_toml, "version")
        .ok_or("Cargo.toml is missing workspace.package version")?;
    let package_version =
        json_string_value(&package_json, "version").ok_or("package.json is missing version")?;
    if package_version != workspace_version {
        return Err(format!(
            "package.json version ({package_version}) must match Cargo.toml workspace version ({workspace_version})"
        )
        .into());
    }

    let cef_version =
        lock_package_version(&cargo_lock, "cef").ok_or("Cargo.lock is missing cef package")?;
    let cef_dll_version = lock_package_version(&cargo_lock, "cef-dll-sys")
        .ok_or("Cargo.lock is missing cef-dll-sys package")?;
    if cef_dll_version != cef_version {
        return Err(format!(
            "cef-dll-sys version ({cef_dll_version}) must match cef version ({cef_version})"
        )
        .into());
    }

    let cef_runtime = runtime_build_version(cef_version)
        .ok_or("cef package version must include a +runtime build suffix")?;
    let mise_runtime =
        quoted_value(&mise_toml, "CEF_VERSION").ok_or("mise.toml is missing CEF_VERSION")?;
    if mise_runtime != cef_runtime {
        return Err(format!(
            "mise.toml CEF_VERSION ({mise_runtime}) must match cef runtime build ({cef_runtime})"
        )
        .into());
    }

    let export_cef_dir_version = quoted_value(&mise_toml, "\"cargo:export-cef-dir\"")
        .ok_or("mise.toml is missing cargo:export-cef-dir tool pin")?;
    if export_cef_dir_version != cef_version {
        return Err(format!(
            "mise.toml cargo:export-cef-dir ({export_cef_dir_version}) must match cef version ({cef_version})"
        )
        .into());
    }

    println!("Version validation complete.");
    Ok(())
}

fn quoted_value<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents.lines().find_map(|line| {
        let line = line.trim();
        let (line_key, value) = line.split_once('=')?;
        if line_key.trim() != key {
            return None;
        }
        value.trim().trim_matches('"').split_whitespace().next()
    })
}

fn json_string_value<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    let json_key = format!("\"{key}\"");
    contents.lines().find_map(|line| {
        let line = line.trim();
        if !line.starts_with(&json_key) {
            return None;
        }
        let (_, value) = line.split_once(':')?;
        value
            .trim()
            .trim_end_matches(',')
            .trim()
            .trim_matches('"')
            .split_whitespace()
            .next()
    })
}

fn lock_package_version<'a>(contents: &'a str, package_name: &str) -> Option<&'a str> {
    let mut in_package = false;
    let mut name_matches = false;

    for line in contents.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            in_package = true;
            name_matches = false;
            continue;
        }

        if !in_package {
            continue;
        }

        if let Some(name) = quoted_assignment(line, "name") {
            name_matches = name == package_name;
            continue;
        }

        if name_matches && let Some(version) = quoted_assignment(line, "version") {
            return Some(version);
        }
    }

    None
}

fn quoted_assignment<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (line_key, value) = line.split_once('=')?;
    if line_key.trim() == key {
        Some(value.trim().trim_matches('"'))
    } else {
        None
    }
}

fn runtime_build_version(version: &str) -> Option<&str> {
    version.split_once('+').map(|(_, runtime)| runtime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_lock_package_version() {
        let lock = r#"
[[package]]
name = "other"
version = "1.0.0"

[[package]]
name = "cef"
version = "148.1.0+147.0.14"
"#;

        assert_eq!(lock_package_version(lock, "cef"), Some("148.1.0+147.0.14"));
    }

    #[test]
    fn extracts_runtime_build_suffix() {
        assert_eq!(runtime_build_version("148.1.0+147.0.14"), Some("147.0.14"));
    }

    #[test]
    fn extracts_quoted_toml_value() {
        let toml = r#"
[env]
CEF_VERSION = "147.0.14"
"#;

        assert_eq!(quoted_value(toml, "CEF_VERSION"), Some("147.0.14"));
    }

    #[test]
    fn extracts_json_string_value() {
        let json = r#"
{
  "name": "godot-cef",
  "version": "1.13.1",
  "license": "MIT"
}
"#;

        assert_eq!(json_string_value(json, "version"), Some("1.13.1"));
    }
}
