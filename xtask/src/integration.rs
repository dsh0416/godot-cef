//! Build, stage and supervise the test-only Godot extension.

mod process;
mod report;
mod server;

use crate::bundle_common::{copy_directory, workspace_root};
use clap::Args;
use report::{Case, CaseResult};
use serde::Serialize;
use server::FixtureServer;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args)]
pub struct Options {
    /// Godot 4.5+ executable (use the console executable on Windows)
    #[arg(long)]
    godot: PathBuf,
    /// Complete production addon, including the helper and matching CEF runtime
    #[arg(long)]
    addon: Option<PathBuf>,
    /// Use a prebuilt native test addon instead of building gdcef_itest
    #[arg(long)]
    test_addon: Option<PathBuf>,
    /// Run one Class:case instead of the complete suite
    #[arg(long)]
    case: Option<String>,
    /// Parent directory for retained reports, logs, projects and CEF profiles
    #[arg(long)]
    output: Option<PathBuf>,
    /// Build the test driver in release mode
    #[arg(long)]
    release: bool,
    /// Explicit native Rust target; cross-compiled tests cannot run on this host
    #[arg(long)]
    target: Option<String>,
}

pub fn run(options: Options) -> Result<()> {
    let native_target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => return Err("headless integration tests currently support Linux and Windows".into()),
    };
    if let Some(target) = &options.target
        && target != native_target
    {
        return Err(format!("cannot run {target} integration tests on {native_target}").into());
    }
    let cases = select_cases(options.case.as_deref())?;
    let root = workspace_root();
    // Resolve caller-relative paths before spawning Cargo or Godot from the repo.
    let godot = std::path::absolute(options.godot)?;
    let addon = std::path::absolute(
        options
            .addon
            .unwrap_or_else(|| root.join("addons/godot_cef")),
    )?;
    let output = std::path::absolute(
        options
            .output
            .unwrap_or_else(|| root.join("target/integration")),
    )?;
    let test_addon = match options.test_addon {
        Some(path) => std::path::absolute(path)?,
        None => build_test_addon(&root, native_target, options.release)?,
    };
    let run_root = output.join(format!(
        "{}-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        random_id()?
    ));
    fs::create_dir_all(&run_root)?;
    let mut suite = Suite {
        project: run_root.join("project"),
        run_root,
        godot,
        addon,
        test_addon,
        origin: None,
        godot_headless: true,
        cef_software_rendering: true,
        fake_media_devices: true,
        fake_ui: false,
        requested_cases: cases.len(),
        completed_cases: 0,
        fatal_error: None,
        results: Vec::new(),
        passed: false,
    };
    if let Err(error) = suite.execute(&cases) {
        let error = error.to_string();
        eprintln!("{error}");
        suite.results.push(CaseResult::setup_failure(error.clone()));
        suite.fatal_error = Some(error);
    }
    suite.completed_cases = suite
        .results
        .iter()
        .filter(|item| item.case.class != "harness")
        .count();
    suite.passed = suite.fatal_error.is_none()
        && suite.completed_cases == suite.requested_cases
        && suite.results.iter().all(|item| item.assessment.passed);
    let summary_path = suite.run_root.join("summary.json");
    fs::write(&summary_path, serde_json::to_vec_pretty(&suite)?)?;
    fs::write(
        suite.run_root.join("junit.xml"),
        report::junit(&suite.results)?,
    )?;
    println!("Results: {}", summary_path.display());
    if !suite.passed {
        return Err("headless integration tests failed; see retained reports and logs".into());
    }
    Ok(())
}

fn build_test_addon(root: &Path, native_target: &str, release: bool) -> Result<PathBuf> {
    // Cargo metadata honors CARGO_TARGET_DIR and .cargo/config.toml. Select the
    // native target explicitly to avoid using a stale implicit cross-build.
    #[derive(serde::Deserialize)]
    struct Metadata {
        target_directory: PathBuf,
    }
    let metadata = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()?;
    if !metadata.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&metadata.stderr)
        )
        .into());
    }
    let metadata: Metadata = serde_json::from_slice(&metadata.stdout)?;
    let mut args = vec![
        "build",
        "--locked",
        "--package",
        "gdcef_itest",
        "--target",
        native_target,
    ];
    if release {
        args.push("--release");
    }
    println!("Running: cargo {}", args.join(" "));
    let status = Command::new("cargo")
        .args(args)
        .current_dir(root)
        .status()?;
    if !status.success() {
        return Err(format!("test addon build failed: {status}").into());
    }
    Ok(metadata
        .target_directory
        .join(native_target)
        .join(if release { "release" } else { "debug" })
        .join(library_name())
        .canonicalize()?)
}

const SCENARIOS: &[&str] = &[
    "permission_grant_all",
    "permission_deny_one",
    "permission_timeout",
    "permission_navigation",
    "permission_unhandled_then_listen",
    "js_ipc",
    "lifecycle",
];

fn select_cases(filter: Option<&str>) -> Result<Vec<Case>> {
    let cases: Vec<_> = ["CefTexture", "CefTexture2D"]
        .into_iter()
        .flat_map(|class| SCENARIOS.iter().map(move |&case| Case { class, case }))
        .collect();
    match filter {
        None => Ok(cases),
        Some(filter) => {
            let selected: Vec<_> = cases
                .iter()
                .copied()
                .filter(|item| format!("{}:{}", item.class, item.case) == filter)
                .collect();
            if selected.is_empty() {
                return Err(format!(
                    "Unknown --case {filter}. Cases: {}",
                    cases
                        .iter()
                        .map(|item| format!("{}:{}", item.class, item.case))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .into());
            }
            Ok(selected)
        }
    }
}

fn library_name() -> &'static str {
    if cfg!(windows) {
        "gdcef_itest.dll"
    } else {
        "libgdcef_itest.so"
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Suite {
    #[serde(skip)]
    run_root: PathBuf,
    godot: PathBuf,
    addon: PathBuf,
    test_addon: PathBuf,
    origin: Option<String>,
    project: PathBuf,
    godot_headless: bool,
    cef_software_rendering: bool,
    fake_media_devices: bool,
    fake_ui: bool,
    requested_cases: usize,
    completed_cases: usize,
    fatal_error: Option<String>,
    results: Vec<CaseResult>,
    passed: bool,
}

impl Suite {
    fn stage_project(&self) -> Result<()> {
        if !self.godot.is_file() || !self.test_addon.is_file() {
            return Err("Godot executable and test addon must be existing files".into());
        }
        if !self.addon.join("godot_cef.gdextension").is_file() || !self.addon.join("bin").is_dir() {
            return Err(
                "--addon must contain godot_cef.gdextension and the complete bin bundle".into(),
            );
        }
        fs::create_dir(&self.project)?;
        fs::write(
            self.project.join("project.godot"),
            include_str!("../../tests/integration/project.godot"),
        )?;
        fs::write(
            self.project.join("main.tscn"),
            include_str!("../../tests/integration/main.tscn"),
        )?;
        copy_directory(&self.addon, &self.project.join("addons/godot_cef"))?;
        let test_dir = self.project.join("addons/gdcef_itest");
        fs::create_dir_all(test_dir.join("bin"))?;
        fs::copy(&self.test_addon, test_dir.join("bin").join(library_name()))?;
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x86_64"
        };
        let feature = format!("{}.{arch}", std::env::consts::OS);
        let descriptor = include_str!("../../tests/integration/gdcef_itest.gdextension.in")
            .replace("@FEATURE@", &feature)
            .replace("@LIBRARY@", library_name());
        fs::write(test_dir.join("gdcef_itest.gdextension"), descriptor)?;
        Ok(())
    }

    fn execute(&mut self, cases: &[Case]) -> Result<()> {
        self.stage_project()?;
        for (name, arguments) in [
            (
                "import",
                vec!["--headless", "--editor", "--import", "--quit"],
            ),
            ("startup", vec!["--headless", "--quit-after", "3"]),
        ] {
            let mut command = self.command();
            command.args(arguments).env("GDCEF_ITEST", "0");
            let result = self.invoke(&mut command, name, None)?;
            if !self.record(name, result)? {
                return Err(format!("Godot {name} smoke failed").into());
            }
        }
        let server =
            FixtureServer::start(include_bytes!("../../tests/integration/page.html").to_vec())?;
        self.origin = Some(server.origin().to_owned());
        for &case in cases {
            let name = format!("{}-{}", case.class, case.case);
            let id = format!("{name}-{}", random_id()?);
            server.register(&id);
            let mut command = self.command();
            command
                .arg("--headless")
                .env("GDCEF_ITEST", "1")
                .env("GDCEF_ITEST_CLASS", case.class)
                .env("GDCEF_ITEST_CASE", case.case)
                .env("GDCEF_ITEST_ORIGIN", server.origin())
                .env("GDCEF_ITEST_RUN", &id)
                .env(
                    "GDCEF_ITEST_PROFILE",
                    self.run_root.join(&name).join("cef-profile"),
                );
            let mut result = self.invoke(&mut command, &name, Some(case))?;
            result.browser_events = server.events(&id);
            self.record(&name, result)?;
        }
        Ok(())
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.godot);
        command
            .current_dir(&self.project)
            .arg("--path")
            .arg(&self.project);
        command
    }

    fn invoke(
        &self,
        command: &mut Command,
        name: &str,
        expected: Option<Case>,
    ) -> Result<CaseResult> {
        let case_dir = self.run_root.join(name);
        fs::create_dir(&case_dir)?;
        let log = case_dir.join("godot.log");
        let outcome = process::run(command, &log, Duration::from_secs(60))?;
        let assessment = report::assess(&outcome, expected);
        // Smoke names come from the fixed list above; normal cases retain their name.
        let case = expected.unwrap_or(Case {
            class: "harness",
            case: if name == "import" {
                "import"
            } else {
                "startup"
            },
        });
        Ok(CaseResult {
            case,
            process: Some(outcome),
            assessment,
            browser_events: Vec::new(),
            log: Some(log),
        })
    }

    fn record(&mut self, name: &str, result: CaseResult) -> Result<bool> {
        fs::write(
            self.run_root.join(name).join("result.json"),
            serde_json::to_vec_pretty(&result)?,
        )?;
        let passed = result.assessment.passed;
        let checks = result
            .assessment
            .report
            .as_ref()
            .and_then(|item| item.get("checks"))
            .map(|checks| format!(" ({checks} checks)"))
            .unwrap_or_default();
        println!(
            "{} {}:{}{checks}",
            if passed { "PASS" } else { "FAIL" },
            result.case.class,
            result.case.case
        );
        for failure in &result.assessment.failures {
            eprintln!("{failure}");
        }
        self.results.push(result);
        Ok(passed)
    }
}

fn random_id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|error| format!("Cannot generate run ID: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
// Assertions intentionally panic; Result propagates fixture setup errors.
#[allow(clippy::panic_in_result_fn)]
mod tests {
    use super::*;

    #[test]
    fn selection_preserves_full_matrix_and_rejects_unknown_cases() -> Result<()> {
        assert_eq!(select_cases(None)?.len(), 14);
        let selected = select_cases(Some("CefTexture2D:permission_navigation"))?;
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].class, "CefTexture2D");
        assert!(select_cases(Some("CefTexture2D:typo")).is_err());
        Ok(())
    }
}
