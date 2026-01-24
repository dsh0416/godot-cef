use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(target_os = "macos")]
use serde::Serialize;
#[cfg(target_os = "macos")]
use std::collections::HashMap;

#[cfg(target_os = "macos")]
#[derive(Serialize)]
pub struct AppInfoPlist {
    #[serde(rename = "CFBundleDevelopmentRegion")]
    pub cf_bundle_development_region: String,
    #[serde(rename = "CFBundleDisplayName")]
    pub cf_bundle_display_name: String,
    #[serde(rename = "CFBundleExecutable")]
    pub cf_bundle_executable: String,
    #[serde(rename = "CFBundleIdentifier")]
    pub cf_bundle_identifier: String,
    #[serde(rename = "CFBundleInfoDictionaryVersion")]
    pub cf_bundle_info_dictionary_version: String,
    #[serde(rename = "CFBundleName")]
    pub cf_bundle_name: String,
    #[serde(rename = "CFBundlePackageType")]
    pub cf_bundle_package_type: String,
    #[serde(rename = "CFBundleSignature")]
    pub cf_bundle_signature: String,
    #[serde(rename = "CFBundleVersion")]
    pub cf_bundle_version: String,
    #[serde(rename = "CFBundleShortVersionString")]
    pub cf_bundle_short_version_string: String,
    #[serde(rename = "LSEnvironment")]
    pub ls_environment: HashMap<String, String>,
    #[serde(rename = "LSFileQuarantineEnabled")]
    pub ls_file_quarantine_enabled: bool,
    #[serde(rename = "LSMinimumSystemVersion")]
    pub ls_minimum_system_version: String,
    #[serde(rename = "LSUIElement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ls_ui_element: Option<String>,
    #[serde(rename = "NSBluetoothAlwaysUsageDescription")]
    pub ns_bluetooth_always_usage_description: String,
    #[serde(rename = "NSSupportsAutomaticGraphicsSwitching")]
    pub ns_supports_automatic_graphics_switching: bool,
    #[serde(rename = "NSWebBrowserPublicKeyCredentialUsageDescription")]
    pub ns_web_browser_publickey_credential_usage_description: String,
    #[serde(rename = "NSCameraUsageDescription")]
    pub ns_camera_usage_description: String,
    #[serde(rename = "NSMicrophoneUsageDescription")]
    pub ns_microphone_usage_description: String,
}

#[cfg(target_os = "macos")]
#[derive(Serialize)]
pub struct FrameworkInfoPlist {
    #[serde(rename = "CFBundleDevelopmentRegion")]
    pub cf_bundle_development_region: String,
    #[serde(rename = "CFBundleExecutable")]
    pub cf_bundle_executable: String,
    #[serde(rename = "CFBundleIdentifier")]
    pub cf_bundle_identifier: String,
    #[serde(rename = "CFBundleInfoDictionaryVersion")]
    pub cf_bundle_info_dictionary_version: String,
    #[serde(rename = "CFBundleName")]
    pub cf_bundle_name: String,
    #[serde(rename = "CFBundlePackageType")]
    pub cf_bundle_package_type: String,
    #[serde(rename = "CFBundleSignature")]
    pub cf_bundle_signature: String,
    #[serde(rename = "CFBundleVersion")]
    pub cf_bundle_version: String,
    #[serde(rename = "CFBundleShortVersionString")]
    pub cf_bundle_short_version_string: String,
    #[serde(rename = "LSEnvironment")]
    pub ls_environment: HashMap<String, String>,
    #[serde(rename = "LSFileQuarantineEnabled")]
    pub ls_file_quarantine_enabled: bool,
    #[serde(rename = "LSMinimumSystemVersion")]
    pub ls_minimum_system_version: String,
    #[serde(rename = "LSUIElement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ls_ui_element: Option<String>,
}

#[cfg(target_os = "macos")]
impl AppInfoPlist {
    pub fn new(exec_name: &str, is_helper: bool) -> Self {
        Self {
            cf_bundle_development_region: "en".to_string(),
            cf_bundle_display_name: exec_name.to_string(),
            cf_bundle_executable: exec_name.to_string(),
            cf_bundle_identifier: "me.delton.gdcef.helper".to_string(),
            cf_bundle_info_dictionary_version: "6.0".to_string(),
            cf_bundle_name: "gdcef".to_string(),
            cf_bundle_package_type: "APPL".to_string(),
            cf_bundle_signature: "????".to_string(),
            cf_bundle_version: "1.0.0".to_string(),
            cf_bundle_short_version_string: "1.0".to_string(),
            ls_environment: [("MallocNanoZone".to_string(), "0".to_string())]
                .iter()
                .cloned()
                .collect(),
            ls_file_quarantine_enabled: true,
            ls_minimum_system_version: "11.0".to_string(),
            ls_ui_element: if is_helper {
                Some("1".to_string())
            } else {
                None
            },
            ns_bluetooth_always_usage_description: exec_name.to_string(),
            ns_supports_automatic_graphics_switching: true,
            ns_web_browser_publickey_credential_usage_description: exec_name.to_string(),
            ns_camera_usage_description: exec_name.to_string(),
            ns_microphone_usage_description: exec_name.to_string(),
        }
    }
}

#[cfg(target_os = "macos")]
impl FrameworkInfoPlist {
    pub fn new(lib_name: &str) -> Self {
        Self {
            cf_bundle_development_region: "en".to_string(),
            cf_bundle_executable: lib_name.to_string(),
            cf_bundle_identifier: "me.delton.gdcef.libgdcef".to_string(),
            cf_bundle_info_dictionary_version: "6.0".to_string(),
            cf_bundle_name: "gdcef".to_string(),
            cf_bundle_package_type: "FMWK".to_string(),
            cf_bundle_signature: "????".to_string(),
            cf_bundle_version: "1.0.0".to_string(),
            cf_bundle_short_version_string: "1.0".to_string(),
            ls_environment: [("MallocNanoZone".to_string(), "0".to_string())]
                .iter()
                .cloned()
                .collect(),
            ls_file_quarantine_enabled: true,
            ls_minimum_system_version: "11.0".to_string(),
            ls_ui_element: None,
        }
    }
}

pub fn copy_directory(src: &Path, dst: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

pub fn run_cargo(args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    println!("Running: cargo {}", args.join(" "));
    let status = Command::new("cargo")
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if !status.success() {
        return Err(format!("cargo {} failed with status: {}", args.join(" "), status).into());
    }
    Ok(())
}

pub fn get_target_dir(release: bool, custom_target_dir: Option<&Path>) -> PathBuf {
    let profile = if release { "release" } else { "debug" };
    let base = custom_target_dir.map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask should be in workspace")
            .join("target")
    });
    base.join(profile)
}

#[cfg(not(target_os = "macos"))]
pub fn get_cef_dir() -> Option<PathBuf> {
    env::var("CEF_PATH").ok().map(PathBuf::from)
}

#[cfg(target_os = "macos")]
pub fn get_cef_dir_arm64() -> Option<PathBuf> {
    env::var("CEF_PATH_ARM64").ok().map(PathBuf::from)
}

#[cfg(target_os = "macos")]
pub fn get_cef_dir_x64() -> Option<PathBuf> {
    env::var("CEF_PATH_X64").ok().map(PathBuf::from)
}

#[cfg(target_os = "macos")]
pub fn get_target_dir_for_target(
    release: bool,
    target: &str,
    custom_target_dir: Option<&Path>,
) -> PathBuf {
    let profile = if release { "release" } else { "debug" };
    let base = custom_target_dir.map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask should be in workspace")
            .join("target")
    });
    base.join(target).join(profile)
}

#[cfg(target_os = "macos")]
pub fn run_lipo(
    input1: &Path,
    input2: &Path,
    output: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "Running: lipo -create {} {} -output {}",
        input1.display(),
        input2.display(),
        output.display()
    );

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let status = Command::new("lipo")
        .arg("-create")
        .arg(input1)
        .arg(input2)
        .arg("-output")
        .arg(output)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    if !status.success() {
        return Err(format!("lipo failed with status: {}", status).into());
    }
    Ok(())
}

pub fn get_addon_bin_dir(platform_target: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask should be in workspace")
        .join("addons/godot_cef/bin")
        .join(platform_target)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn deploy_to_addon(
    source_dir: &Path,
    platform_target: &str,
    files: &[&str],
    dirs: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    let addon_bin_dir = get_addon_bin_dir(platform_target);

    println!("Deploying to addon: {}", addon_bin_dir.display());

    if addon_bin_dir.exists() {
        fs::remove_dir_all(&addon_bin_dir)?;
    }
    fs::create_dir_all(&addon_bin_dir)?;

    for file in files {
        let src = source_dir.join(file);
        let dst = addon_bin_dir.join(file);

        if src.exists() {
            fs::copy(&src, &dst)?;
            println!("  Deployed: {}", file);
        }
    }

    for dir in dirs {
        let src = source_dir.join(dir);
        let dst = addon_bin_dir.join(dir);

        if src.exists() {
            copy_directory(&src, &dst)?;
            println!("  Deployed directory: {}", dir);
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
pub fn deploy_bundle_to_addon(
    bundle_path: &Path,
    platform_target: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let addon_bin_dir = get_addon_bin_dir(platform_target);

    println!("Deploying bundle to addon: {}", addon_bin_dir.display());

    fs::create_dir_all(&addon_bin_dir)?;

    let bundle_name = bundle_path.file_name().ok_or("Invalid bundle path")?;
    let dst = addon_bin_dir.join(bundle_name);

    if dst.exists() {
        fs::remove_dir_all(&dst)?;
    }
    copy_directory(bundle_path, &dst)?;
    println!("  Deployed: {}", bundle_name.to_string_lossy());

    Ok(())
}
