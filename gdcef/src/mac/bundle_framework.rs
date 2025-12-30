// Modified from https://github.com/tauri-apps/cef-rs/blob/dev/examples/cefsimple/src/mac/bundle_cefsimple.rs

#[cfg(target_os = "macos")]
mod mac {
    use serde::Serialize;
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    #[derive(Serialize)]
    struct InfoPlist {
        #[serde(rename = "CFBundleDevelopmentRegion")]
        cf_bundle_development_region: String,
        #[serde(rename = "CFBundleExecutable")]
        cf_bundle_executable: String,
        #[serde(rename = "CFBundleIdentifier")]
        cf_bundle_identifier: String,
        #[serde(rename = "CFBundleInfoDictionaryVersion")]
        cf_bundle_info_dictionary_version: String,
        #[serde(rename = "CFBundleName")]
        cf_bundle_name: String,
        #[serde(rename = "CFBundlePackageType")]
        cf_bundle_package_type: String,
        #[serde(rename = "CFBundleSignature")]
        cf_bundle_signature: String,
        #[serde(rename = "CFBundleVersion")]
        cf_bundle_version: String,
        #[serde(rename = "CFBundleShortVersionString")]
        cf_bundle_short_version_string: String,
        #[serde(rename = "LSEnvironment")]
        ls_environment: HashMap<String, String>,
        #[serde(rename = "LSFileQuarantineEnabled")]
        ls_file_quarantine_enabled: bool,
        #[serde(rename = "LSMinimumSystemVersion")]
        ls_minimum_system_version: String,
        #[serde(rename = "LSUIElement")]
        ls_ui_element: Option<String>
    }

    const RESOURCES_PATH: &str = "Resources";

    fn create_app_layout(app_path: &Path) -> PathBuf {
        [RESOURCES_PATH]
            .iter()
            .for_each(|p| fs::create_dir_all(app_path.join(p)).unwrap());
        app_path.join(RESOURCES_PATH)
    }

    fn create_framework(fmwk_path: &Path, lib_name: &str, bin: &Path) {
        let fmwk_path = fmwk_path.join("Godot CEF.framework");
        let resources_path = create_app_layout(&fmwk_path);
        create_info_plist(&resources_path, "libgdcef.dylib", false).unwrap();
        fs::copy(bin, fmwk_path.join(lib_name)).unwrap();
    }

    // See https://bitbucket.org/chromiumembedded/cef/wiki/GeneralUsage.md#markdown-header-macos
    fn bundle(fmwk_path: &Path) {
        let example_path = PathBuf::from(fmwk_path);
        create_framework(
            fmwk_path,
            "libgdcef.dylib",
            &example_path.join("libgdcef.dylib"),
        );
    }

    fn create_info_plist(
        resources_path: &Path,
        lib_name: &str,
        is_helper: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let info_plist = InfoPlist {
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
            ls_ui_element: if is_helper {
                Some("1".to_string())
            } else {
                None
            }
        };

        plist::to_file_xml(resources_path.join("Info.plist"), &info_plist)?;
        Ok(())
    }

    fn run_command(args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
        let status = Command::new("cargo")
            .args(args)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()?;

        if !status.success() {
            std::process::exit(1);
        }
        Ok(())
    }

    pub fn main() -> Result<(), Box<dyn std::error::Error>> {
        let fmwk_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug");
        run_command(&["build", "--lib"])?;
        bundle(&fmwk_path);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    mac::main()
}

#[cfg(not(target_os = "macos"))]
fn main() {}
