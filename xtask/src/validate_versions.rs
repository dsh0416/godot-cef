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

    let (cef_core, cef_runtime) = locked_cef_version(&cargo_lock)?;
    let export_cef_dir_version = quoted_value(&mise_toml, "\"cargo:export-cef-dir\"")
        .ok_or("mise.toml is missing cargo:export-cef-dir tool pin")?;
    validate_exporter_version(export_cef_dir_version, cef_core, cef_runtime)?;

    println!("Version validation complete.");
    Ok(())
}

pub fn print_cef_version() -> Result<(), Box<dyn std::error::Error>> {
    let cargo_lock = fs::read_to_string(workspace_root().join("Cargo.lock"))?;
    let (_, runtime) = locked_cef_version(&cargo_lock)?;
    println!("{runtime}");
    Ok(())
}

fn quoted_value<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents
        .lines()
        .find_map(|line| quoted_assignment(line.trim(), key))
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

fn lock_package_version<'a>(contents: &'a str, package_name: &str) -> Result<&'a str, String> {
    let mut lines = contents.lines().map(str::trim).peekable();
    let mut count = 0;
    let mut version = None;

    while let Some(line) = lines.next() {
        if line != "[[package]]" {
            continue;
        }

        let mut name = None;
        let mut package_version = None;
        while let Some(&line) = lines.peek() {
            if line.starts_with('[') {
                break;
            }
            lines.next();
            if let Some(value) = quoted_assignment(line, "name") {
                name = Some(value);
            } else if let Some(value) = quoted_assignment(line, "version") {
                package_version = Some(value);
            }
        }

        if name == Some(package_name) {
            count += 1;
            version = package_version;
        }
    }

    if count != 1 {
        return Err(format!(
            "Cargo.lock must contain exactly one {package_name} package; found {count}"
        ));
    }
    version.ok_or_else(|| format!("Cargo.lock {package_name} package is missing version"))
}

fn quoted_assignment<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (line_key, value) = line.split_once('=')?;
    if line_key.trim() == key {
        value
            .trim()
            .strip_prefix('"')?
            .split_once('"')
            .map(|(value, _)| value)
    } else {
        None
    }
}

fn locked_cef_version(contents: &str) -> Result<(&str, &str), String> {
    let cef = lock_package_version(contents, "cef")?;
    let cef_dll = lock_package_version(contents, "cef-dll-sys")?;
    if cef != cef_dll {
        return Err(format!(
            "cef-dll-sys version ({cef_dll}) must match cef version ({cef})"
        ));
    }

    let (core, runtime) = cef
        .split_once('+')
        .ok_or("cef package version must include a +runtime build suffix")?;
    if !numeric_version(core) || !numeric_version(runtime) {
        return Err(format!(
            "cef package version ({cef}) must contain numeric major.minor.patch versions before and after '+'"
        ));
    }
    Ok((core, runtime))
}

fn numeric_version(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
        })
}

fn validate_exporter_version(version: &str, core: &str, runtime: &str) -> Result<(), String> {
    let (exporter_core, exporter_runtime) = version
        .split_once('+')
        .map_or((version, None), |(core, runtime)| (core, Some(runtime)));
    if exporter_core != core || exporter_runtime.is_some_and(|value| value != runtime) {
        return Err(format!(
            "mise.toml cargo:export-cef-dir ({version}) must match cef version ({core}+{runtime}); the runtime suffix may be omitted"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock(cef: &str, cef_dll: &str) -> String {
        format!(
            "[[package]]\nname = \"cef\"\nversion = \"{cef}\"\n\n[[package]]\nname = \"cef-dll-sys\"\nversion = \"{cef_dll}\"\n"
        )
    }

    #[test]
    fn extracts_locked_core_and_runtime() {
        assert_eq!(
            locked_cef_version(&lock("154.2.0+154.0.28", "154.2.0+154.0.28")),
            Ok(("154.2.0", "154.0.28"))
        );
    }

    #[test]
    fn parses_lock_package_tables() {
        let contents = "[[package]]\nversion = \"154.2.0+154.0.28\"\nname = \"cef\"\ndependencies = [\n \"cef-dll-sys\",\n]\n[metadata]\nname = \"cef\"\nversion = \"other\"\n";
        assert_eq!(
            lock_package_version(contents, "cef"),
            Ok("154.2.0+154.0.28")
        );
        assert!(lock_package_version("[[package]]\nname = \"cef\"\n", "cef").is_err());
    }

    #[test]
    fn requires_one_package_of_each_name() {
        let valid = lock("154.2.0+154.0.28", "154.2.0+154.0.28");
        for name in ["cef", "cef-dll-sys"] {
            let missing = valid.replace(&format!("name = \"{name}\""), "name = \"other\"");
            assert!(locked_cef_version(&missing).is_err());
            let duplicate = format!(
                "{valid}\n[[package]]\nname = \"{name}\"\nversion = \"154.2.0+154.0.28\"\n"
            );
            assert!(locked_cef_version(&duplicate).is_err());
        }
    }

    #[test]
    fn requires_complete_locked_version_agreement() {
        for other in ["154.3.0+154.0.28", "154.2.0+154.0.32"] {
            assert!(locked_cef_version(&lock("154.2.0+154.0.28", other)).is_err());
        }
    }

    #[test]
    fn rejects_invalid_core_or_runtime() {
        for version in [
            "154.2.0",
            "154.2.0+",
            "154.2.0+154.0",
            "154.2.0+154.0.28.extra",
            "154.2.0+154.0.x",
            "154.2.0+154.0.028",
            "154.2+154.0.28",
            "154.2.0-beta+154.0.28",
            "154.02.0+154.0.28",
        ] {
            assert!(
                locked_cef_version(&lock(version, version)).is_err(),
                "{version}"
            );
        }
    }

    #[test]
    fn exporter_matches_full_core_and_optional_runtime() {
        for version in ["154.2.0", "154.2.0+154.0.28"] {
            assert!(validate_exporter_version(version, "154.2.0", "154.0.28").is_ok());
        }
        for version in ["154.3.0", "154.2.1", "154.2.0+154.0.32", "154.2.0+"] {
            assert!(validate_exporter_version(version, "154.2.0", "154.0.28").is_err());
        }
    }

    #[test]
    fn parses_quoted_toml_assignments() {
        assert_eq!(
            quoted_value(
                "\"cargo:export-cef-dir\" = \"154.2.0\" # comment",
                "\"cargo:export-cef-dir\""
            ),
            Some("154.2.0")
        );
        assert_eq!(quoted_value("version = 154.2.0", "version"), None);
        assert_eq!(quoted_value("version = \"154.2.0", "version"), None);
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
