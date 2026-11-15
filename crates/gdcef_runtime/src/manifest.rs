use crate::{
    Result, fail,
    helper::{reject_links, safe_relative, temporary_path},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub const MANIFEST_FILE: &str = "runtime-install.json";
pub(crate) const RECEIPT_FILE: &str = ".runtime-installed.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmbeddedArchive {
    pub offset: u64,
    pub length: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Platform {
    pub target: String,
    pub directory: String,
    pub library: String,
    pub bootstrap: String,
    pub helper: EmbeddedArchive,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub dirs: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub plugin_version: String,
    pub cef_version: String,
    pub platforms: Vec<Platform>,
}

#[derive(Clone, Debug)]
pub struct TargetStatus {
    pub target: String,
    pub installed: bool,
    pub needs_signing: bool,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub schema_version: u32,
    pub plugin_version: String,
    pub cef_version: String,
    pub helper_sha256: String,
    #[serde(default)]
    pub signing_pending: bool,
}

impl Manifest {
    pub fn load(addon: &Path) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(&fs::read(addon.join(MANIFEST_FILE))?)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(fail("Unsupported runtime installation manifest version"));
        }
        for version in [&self.plugin_version, &self.cef_version] {
            if version.is_empty()
                || !version
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || ".+-".contains(c))
            {
                return Err(fail("Invalid runtime version"));
            }
        }
        if self.platforms.is_empty() {
            return Err(fail("No platforms in the runtime manifest"));
        }
        let mut targets = std::collections::HashSet::new();
        for platform in &self.platforms {
            if !targets.insert(&platform.target) {
                return Err(fail("Duplicate platform in the runtime manifest"));
            }
            feature_tag(&platform.target)?;
            let directory = safe_relative(&platform.directory)?;
            for path in [&platform.library, &platform.bootstrap] {
                let relative = safe_relative(path)?;
                if !relative.starts_with(&directory) || relative == directory {
                    return Err(fail("Native library must be inside its platform directory"));
                }
            }
            for path in platform.files.iter().chain(&platform.dirs) {
                safe_relative(path)?;
            }
            if platform.helper.length == 0 || platform.helper.length > 256 * 1024 * 1024 {
                return Err(fail("Invalid embedded helper length"));
            }
            if platform.helper.sha256.len() != 64
                || !platform
                    .helper
                    .sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit())
            {
                return Err(fail("Invalid embedded helper checksum"));
            }
        }
        Ok(())
    }

    pub fn platform(&self, target: &str) -> Result<&Platform> {
        self.platforms
            .iter()
            .find(|platform| platform.target == target)
            .ok_or_else(|| fail(format!("This package does not support {target}")))
    }
}

pub fn host_target() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "universal-apple-darwin"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "aarch64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(not(any(
        target_os = "macos",
        all(
            any(target_os = "windows", target_os = "linux"),
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    )))]
    {
        "unsupported"
    }
}

pub fn platform_directory(addon: &Path, platform: &Platform) -> Result<PathBuf> {
    let directory = safe_relative(&platform.directory)?;
    reject_links(addon, &directory)?;
    Ok(addon.join(directory))
}

pub(crate) fn helper_payload(addon: &Path, platform: &Platform) -> Result<Vec<u8>> {
    let library = safe_relative(&platform.library)?;
    reject_links(addon, &library)?;
    let mut file = fs::File::open(addon.join(library))?;
    let end = platform
        .helper
        .offset
        .checked_add(platform.helper.length)
        .ok_or_else(|| fail("Invalid helper offset"))?;
    let within_file = end <= file.metadata()?.len();
    if !within_file && platform.target != "universal-apple-darwin" {
        return Err(fail("Embedded helper lies outside the native library"));
    }
    let mut bytes = vec![
        0_u8;
        usize::try_from(platform.helper.length)
            .map_err(|_| fail("Helper is too large"))?
    ];
    if within_file {
        file.seek(SeekFrom::Start(platform.helper.offset))?;
        file.read_exact(&mut bytes)?;
    }
    if crate::sha256(&bytes) != platform.helper.sha256.to_ascii_lowercase() {
        if platform.target == "universal-apple-darwin" {
            // Re-signing a universal Mach-O may shift a later architecture's
            // slice. The compressed payload remains byte-identical; recover its
            // location by checksum without loading a foreign architecture.
            file.seek(SeekFrom::Start(0))?;
            let mut image = Vec::new();
            file.take(512 * 1024 * 1024).read_to_end(&mut image)?;
            let length = bytes.len();
            for (offset, signature) in image.windows(3).enumerate() {
                if signature == [0x1f, 0x8b, 0x08] && offset + length <= image.len() {
                    let candidate = &image[offset..offset + length];
                    if crate::sha256(candidate) == platform.helper.sha256.to_ascii_lowercase() {
                        return Ok(candidate.to_vec());
                    }
                }
            }
        }
        return Err(fail("Embedded helper checksum mismatch"));
    }
    Ok(bytes)
}

pub(crate) fn validate_files(directory: &Path, platform: &Platform) -> Result<()> {
    for name in &platform.files {
        let relative = safe_relative(name)?;
        // CEF frameworks may contain their upstream relative symlinks.
        if !directory.join(relative).is_file() {
            return Err(fail(format!("Missing runtime file: {name}")));
        }
    }
    for name in &platform.dirs {
        if !directory.join(safe_relative(name)?).is_dir() {
            return Err(fail(format!("Missing runtime directory: {name}")));
        }
    }
    if platform.files.is_empty() {
        return Err(fail("Runtime manifest has no required files"));
    }
    Ok(())
}

#[cfg(any(feature = "installer", test))]
pub(crate) fn receipt(manifest: &Manifest, platform: &Platform, signing_pending: bool) -> Receipt {
    Receipt {
        schema_version: 1,
        plugin_version: manifest.plugin_version.clone(),
        cef_version: manifest.cef_version.clone(),
        helper_sha256: platform.helper.sha256.clone(),
        signing_pending,
    }
}

fn read_receipt(
    addon: &Path,
    manifest: &Manifest,
    platform: &Platform,
) -> Result<(PathBuf, Receipt)> {
    let directory = platform_directory(addon, platform)?;
    let installed: Receipt = serde_json::from_slice(&fs::read(directory.join(RECEIPT_FILE))?)?;
    if installed.schema_version != 1
        || installed.plugin_version != manifest.plugin_version
        || installed.cef_version != manifest.cef_version
        || installed.helper_sha256 != platform.helper.sha256
    {
        return Err(fail(
            "Installed runtime belongs to a different plugin or CEF version",
        ));
    }
    Ok((directory, installed))
}

fn target_status(addon: &Path, manifest: &Manifest, platform: &Platform) -> Result<TargetStatus> {
    let (directory, installed) = read_receipt(addon, manifest, platform)?;
    validate_files(&directory, platform)?;
    if !addon.join(&platform.library).is_file() {
        return Err(fail("Missing browser native library"));
    }
    Ok(TargetStatus {
        target: platform.target.clone(),
        installed: !installed.signing_pending,
        needs_signing: installed.signing_pending,
        message: if installed.signing_pending {
            "Downloaded; open the project on macOS to finish signing"
        } else {
            "Installed"
        }
        .to_owned(),
    })
}

pub fn statuses(addon: &Path, manifest: &Manifest) -> Vec<TargetStatus> {
    manifest
        .platforms
        .iter()
        .map(|platform| {
            target_status(addon, manifest, platform).unwrap_or_else(|error| TargetStatus {
                target: platform.target.clone(),
                installed: false,
                needs_signing: false,
                message: error.to_string(),
            })
        })
        .collect()
}

fn feature_tag(target: &str) -> Result<&'static str> {
    match target {
        "x86_64-pc-windows-msvc" => Ok("windows.x86_64"),
        "aarch64-pc-windows-msvc" => Ok("windows.arm64"),
        "x86_64-unknown-linux-gnu" => Ok("linux.x86_64"),
        "aarch64-unknown-linux-gnu" => Ok("linux.arm64"),
        "universal-apple-darwin" => Ok("macos"),
        _ => Err(fail(format!("Unsupported runtime platform: {target}"))),
    }
}

pub(crate) fn replace_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| fail("File has no parent"))?;
    let name = path.file_name().ok_or_else(|| fail("File has no name"))?;
    reject_links(parent, Path::new(name))?;
    if path.exists() && !path.is_file() {
        return Err(fail(format!(
            "Refusing to replace a non-file runtime descriptor: {}",
            path.display()
        )));
    }
    if path.is_file() && fs::read(path)? == bytes {
        return Ok(());
    }
    let temp = temporary_path(parent, "gdcef-install");
    let backup = temporary_path(parent, "gdcef-previous");
    fs::write(&temp, bytes)?;
    if path.exists()
        && let Err(error) = fs::rename(path, &backup)
    {
        let _ = fs::remove_file(temp);
        return Err(error.into());
    }
    match fs::rename(&temp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(error) => {
            if backup.exists() {
                let _ = fs::rename(&backup, path);
            }
            let _ = fs::remove_file(temp);
            Err(error.into())
        }
    }
}

fn ready_platforms<'a>(addon: &Path, manifest: &'a Manifest) -> Vec<&'a Platform> {
    let statuses = statuses(addon, manifest);
    manifest
        .platforms
        .iter()
        .zip(statuses)
        .filter_map(|(platform, state)| state.installed.then_some(platform))
        .collect()
}

fn core_library(platform: &Platform) -> String {
    if platform.target == "universal-apple-darwin" {
        format!("{}/Godot CEF.framework", platform.directory)
    } else {
        platform.library.clone()
    }
}

fn descriptor_string_value(content: &str, section: &str, key: &str) -> Option<String> {
    let mut active_section = false;
    for line in content.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            active_section = &line[1..line.len() - 1] == section;
        } else if active_section
            && let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            return serde_json::from_str(value.trim()).ok();
        }
    }
    None
}

fn descriptor_has_core(content: &str, platforms: &[&Platform]) -> Result<bool> {
    if descriptor_string_value(content, "configuration", "entry_symbol").as_deref()
        != Some("gdext_rust_init")
    {
        return Ok(false);
    }
    for platform in platforms {
        if descriptor_string_value(content, "libraries", feature_tag(&platform.target)?)
            != Some(core_library(platform))
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Check whether ready runtimes require activation at the next editor shutdown.
/// This does not publish a descriptor or modify Godot's extension list. Even an
/// inert new descriptor triggers Godot 4.6's asynchronous documentation rebuild,
/// whose deferred callbacks can race editor shutdown (Godot issue #111048).
pub fn defer_extension_activation(addon: &Path, manifest: &Manifest) -> Result<bool> {
    manifest.validate()?;
    let ready = ready_platforms(addon, manifest);
    if !ready
        .iter()
        .any(|platform| platform.target == host_target())
    {
        return Ok(false);
    }
    let descriptor = addon.join("godot_cef.gdextension");
    reject_links(addon, Path::new("godot_cef.gdextension"))?;
    if descriptor.exists() {
        let content = fs::read_to_string(descriptor)?;
        return Ok(!descriptor_has_core(&content, &ready)?);
    }
    Ok(true)
}

fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>> {
    let parent = path.parent().ok_or_else(|| fail("File has no parent"))?;
    let name = path.file_name().ok_or_else(|| fail("File has no name"))?;
    reject_links(parent, Path::new(name))?;
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn extension_registration(
    extension_list: &Path,
    descriptor_resource_path: &str,
) -> Result<Vec<u8>> {
    if !extension_list.is_absolute()
        || extension_list.file_name().and_then(|name| name.to_str()) != Some("extension_list.cfg")
    {
        return Err(fail(
            "Expected the editor-provided absolute extension list path",
        ));
    }
    let relative = descriptor_resource_path
        .strip_prefix("res://")
        .ok_or_else(|| fail("Expected a project resource path for the browser extension"))?;
    if descriptor_resource_path.contains(['\r', '\n'])
        || descriptor_resource_path.trim() != descriptor_resource_path
        || safe_relative(relative)?
            .file_name()
            .and_then(|name| name.to_str())
            != Some("godot_cef.gdextension")
    {
        return Err(fail("Invalid browser extension resource path"));
    }
    let original = read_optional_file(extension_list)?.unwrap_or_default();
    let mut content = String::from_utf8(original)
        .map_err(|error| fail(format!("Invalid editor extension list: {error}")))?;
    if !content
        .lines()
        .any(|line| line.trim() == descriptor_resource_path)
    {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(descriptor_resource_path);
        content.push('\n');
    }
    Ok(content.into_bytes())
}

/// Check publication inputs and writable directories while installation errors
/// can still be shown in the editor. The finalizer repeats validation at shutdown.
pub fn preflight_extension_activation(
    addon: &Path,
    manifest: &Manifest,
    extension_list: &Path,
    descriptor_resource_path: &str,
) -> Result<()> {
    manifest.validate()?;
    if !ready_platforms(addon, manifest)
        .iter()
        .any(|platform| platform.target == host_target())
    {
        return Ok(());
    }
    extension_registration(extension_list, descriptor_resource_path)?;
    read_optional_file(&addon.join("godot_cef.gdextension"))?;
    let cache_directory = extension_list
        .parent()
        .ok_or_else(|| fail("Extension list has no parent"))?;
    for directory in [addon, cache_directory] {
        let probe = temporary_path(directory, "gdcef-activation-probe");
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        let result = file.sync_all();
        drop(file);
        let removed = fs::remove_file(probe);
        result?;
        removed?;
    }
    Ok(())
}

/// Publish at Scene shutdown, after the editor and scanner have been destroyed.
/// The caller captures the engine's actual extension-list path and localized
/// descriptor path beforehand; this backend never assumes a `.godot` directory.
/// Existing registrations are preserved. If registering the newly published
/// descriptor fails, restore its previous bytes so it cannot be discovered late
/// on the next launch without the matching early-startup registration.
pub fn finalize_extension_activation(
    addon: &Path,
    manifest: &Manifest,
    extension_list: &Path,
    descriptor_resource_path: &str,
) -> Result<()> {
    manifest.validate()?;
    if !ready_platforms(addon, manifest)
        .iter()
        .any(|platform| platform.target == host_target())
    {
        return Ok(());
    }
    let registration = extension_registration(extension_list, descriptor_resource_path)?;
    let descriptor = addon.join("godot_cef.gdextension");
    let previous = read_optional_file(&descriptor)?;
    activate_extension(addon, manifest)?;
    if let Err(error) = replace_file(extension_list, &registration) {
        let restored = match previous {
            Some(bytes) => replace_file(&descriptor, &bytes),
            None => fs::remove_file(&descriptor).map_err(crate::Error::from),
        };
        return match restored {
            Ok(()) => Err(error),
            Err(rollback) => Err(fail(format!(
                "Cannot register browser extension: {error}; restoring the previous descriptor also failed: {rollback}"
            ))),
        };
    }
    Ok(())
}

/// Publish the browser extension only after the editor filesystem scanner has
/// stopped. Never call while an editor can discover and load this descriptor.
/// A previous valid descriptor remains untouched when the host is not ready.
pub fn activate_extension(addon: &Path, manifest: &Manifest) -> Result<()> {
    manifest.validate()?;
    let ready = ready_platforms(addon, manifest);
    if !ready
        .iter()
        .any(|platform| platform.target == host_target())
    {
        return Ok(());
    }
    let mut output = String::from(
        "[configuration]\nentry_symbol = \"gdext_rust_init\"\ncompatibility_minimum = 4.6\nreloadable = false\n\n[libraries]\n",
    );
    for platform in &ready {
        let library = core_library(platform);
        output.push_str(&format!(
            "{} = {}\n",
            feature_tag(&platform.target)?,
            serde_json::to_string(&library)?
        ));
    }
    output.push_str("\n[icons]\nCefTexture = \"icons/webview.png\"\nCefIpcInspector = \"icons/webview.png\"\n\n[dependencies]\n");
    for platform in &ready {
        if platform.target == "universal-apple-darwin" {
            if addon
                .join(&platform.directory)
                .join("cef-licenses")
                .is_dir()
            {
                output.push_str(&format!(
                    "macos = {{ {}: \"\" }}\n",
                    serde_json::to_string(&format!("{}/cef-licenses", platform.directory))?
                ));
            }
            continue;
        }
        output.push_str(&format!("{} = {{\n", feature_tag(&platform.target)?));
        let mut paths = std::collections::BTreeSet::new();
        paths.insert(platform.library.clone());
        for entry in platform.files.iter().chain(&platform.dirs) {
            let path = format!("{}/{}", platform.directory, entry);
            if path != platform.bootstrap {
                paths.insert(path);
            }
        }
        if addon
            .join(&platform.directory)
            .join("cef-licenses")
            .is_dir()
        {
            paths.insert(format!("{}/cef-licenses", platform.directory));
        }
        for path in paths {
            output.push_str(&format!("  {}: \"\",\n", serde_json::to_string(&path)?));
        }
        output.push_str("}\n");
    }
    replace_file(&addon.join("godot_cef.gdextension"), output.as_bytes())
}

pub(crate) fn verify_runtime(addon: &Path, manifest: &Manifest, platform: &Platform) -> Result<()> {
    let (directory, installed) = read_receipt(addon, manifest, platform)?;
    if installed.signing_pending {
        return Err(fail(
            "Open the project on macOS to finish signing this runtime before export",
        ));
    }
    let payload = helper_payload(addon, platform)?;
    if platform.target != "universal-apple-darwin" {
        crate::ensure_helper_payload(&payload, &platform.helper.sha256, &directory)?;
    } else {
        #[cfg(feature = "installer")]
        crate::installer::verify_mac(&directory)?;
    }
    validate_files(&directory, platform)?;
    Ok(())
}

pub fn prepare_export(addon: &Path, manifest: &Manifest, target: &str) -> Result<()> {
    manifest.validate()?;
    let platform = manifest.platform(target)?;
    verify_runtime(addon, manifest, platform)?;
    if !statuses(addon, manifest)
        .iter()
        .any(|state| state.target == host_target() && state.installed)
    {
        return Err(fail(format!(
            "Install the editor platform runtime ({}) before preparing an export",
            host_target()
        )));
    }
    let descriptor = addon.join("godot_cef.gdextension");
    reject_links(addon, Path::new("godot_cef.gdextension"))?;
    let content = fs::read_to_string(descriptor).unwrap_or_default();
    if !descriptor_has_core(&content, &[manifest.platform(host_target())?, platform])? {
        return Err(fail(
            "Runtime installation is ready. Restart the editor before exporting so the browser extension and target dependencies are activated",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::panic_in_result_fn)] // Test assertions deliberately fail the test harness.
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            schema_version: 1,
            plugin_version: "2.1.3".into(),
            cef_version: "156.0.26".into(),
            platforms: vec![Platform {
                target: "x86_64-pc-windows-msvc".into(),
                directory: "bin/win/v1".into(),
                library: "bin/win/v1/gdcef.dll".into(),
                bootstrap: "bin/win/v1/bootstrap.dll".into(),
                helper: EmbeddedArchive {
                    offset: 1,
                    length: 1,
                    sha256: "0".repeat(64),
                },
                files: vec!["libcef.dll".into(), "gdcef_helper.exe".into()],
                dirs: vec!["locales".into()],
            }],
        }
    }

    #[test]
    fn manifest_rejects_escaping_paths_and_unsupported_targets() {
        let mut data = manifest();
        assert!(data.validate().is_ok());
        data.platforms[0].library = "../gdcef.dll".into();
        assert!(data.validate().is_err());
        data = manifest();
        data.platforms[0].files.push("../../outside".into());
        assert!(data.validate().is_err());
    }

    #[test]
    fn activation_requires_matching_receipt_and_complete_dependencies() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-manifest-test");
        fs::create_dir_all(&root)?;
        let mut data = manifest();
        data.platforms[0].target = host_target().to_owned();
        let directory = platform_directory(&root, &data.platforms[0])?;
        fs::create_dir_all(directory.join("locales"))?;
        fs::write(directory.join("gdcef.dll"), b"native")?;
        fs::write(directory.join("libcef.dll"), b"cef")?;
        fs::write(
            directory.join(RECEIPT_FILE),
            serde_json::to_vec(&receipt(&data, &data.platforms[0], false))?,
        )?;
        activate_extension(&root, &data)?;
        assert!(!root.join("godot_cef.gdextension").exists());
        fs::write(directory.join("gdcef_helper.exe"), b"helper")?;
        activate_extension(&root, &data)?;
        let output = fs::read_to_string(root.join("godot_cef.gdextension"))?;
        assert!(output.contains(&format!("{} = ", feature_tag(host_target())?)));
        assert!(!output.contains("bootstrap.dll"));
        fs::write(directory.join(RECEIPT_FILE), b"not json")?;
        assert!(!statuses(&root, &data)[0].installed);
        let mut stale = receipt(&data, &data.platforms[0], false);
        stale.cef_version = "155.0.1".into();
        fs::write(directory.join(RECEIPT_FILE), serde_json::to_vec(&stale)?)?;
        assert!(!statuses(&root, &data)[0].installed);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn deferred_activation_never_exposes_core_until_finalization() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-pending-manifest-test");
        fs::create_dir_all(&root)?;
        let mut data = manifest();
        data.platforms[0].target = host_target().to_owned();
        let directory = platform_directory(&root, &data.platforms[0])?;
        fs::create_dir_all(directory.join("locales"))?;
        for name in [
            "gdcef.dll",
            "libcef.dll",
            "gdcef_helper.exe",
            "bootstrap.dll",
        ] {
            fs::write(directory.join(name), b"fixture")?;
        }
        assert!(!defer_extension_activation(&root, &data)?);
        assert!(!root.join("godot_cef.gdextension").exists());
        fs::write(
            directory.join(RECEIPT_FILE),
            serde_json::to_vec(&receipt(&data, &data.platforms[0], false))?,
        )?;
        assert!(defer_extension_activation(&root, &data)?);
        let path = root.join("godot_cef.gdextension");
        assert!(!path.exists());
        assert!(defer_extension_activation(&root, &data)?);
        assert!(!path.exists());
        let cache_dir = root.join("engine-provided-cache");
        fs::create_dir(&cache_dir)?;
        let extension_list = cache_dir.join("extension_list.cfg");
        let resource = "res://addons/godot_cef/godot_cef.gdextension";
        let others = b"res://addons/other/other.gdextension\r\nres://another.gdextension";
        fs::write(&extension_list, others)?;
        preflight_extension_activation(&root, &data, &extension_list, resource)?;
        assert!(!path.exists());
        assert_eq!(fs::read(&extension_list)?, others);
        finalize_extension_activation(&root, &data, &extension_list, resource)?;
        let registration = fs::read(&extension_list)?;
        assert!(registration.starts_with(others));
        assert_eq!(
            fs::read_to_string(&extension_list)?.lines().last(),
            Some(resource)
        );
        let active = fs::read_to_string(&path)?;
        assert!(descriptor_has_core(&active, &[&data.platforms[0]])?);
        assert!(!defer_extension_activation(&root, &data)?);
        finalize_extension_activation(&root, &data, &extension_list, resource)?;
        assert_eq!(fs::read_to_string(&path)?, active);
        assert_eq!(fs::read(&extension_list)?, registration);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn deferred_upgrade_preserves_active_descriptor_until_finalization() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-pending-upgrade-test");
        fs::create_dir_all(&root)?;
        let mut data = manifest();
        data.platforms[0].target = host_target().to_owned();
        let directory = platform_directory(&root, &data.platforms[0])?;
        fs::create_dir_all(directory.join("locales"))?;
        for name in ["gdcef.dll", "libcef.dll", "gdcef_helper.exe"] {
            fs::write(directory.join(name), b"fixture")?;
        }
        fs::write(
            directory.join(RECEIPT_FILE),
            serde_json::to_vec(&receipt(&data, &data.platforms[0], false))?,
        )?;
        let path = root.join("godot_cef.gdextension");
        fs::write(&path, b"previous working runtime")?;
        assert!(defer_extension_activation(&root, &data)?);
        assert_eq!(fs::read(&path)?, b"previous working runtime");
        activate_extension(&root, &data)?;
        assert!(descriptor_has_core(
            &fs::read_to_string(&path)?,
            &[&data.platforms[0]]
        )?);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn failed_registration_restores_old_descriptor_and_removes_new_descriptor() -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        let root = temporary_path(&std::env::temp_dir(), "gdcef-registration-rollback-test");
        fs::create_dir_all(&root)?;
        let mut data = manifest();
        data.platforms[0].target = host_target().to_owned();
        let directory = platform_directory(&root, &data.platforms[0])?;
        fs::create_dir_all(directory.join("locales"))?;
        for name in ["gdcef.dll", "libcef.dll", "gdcef_helper.exe"] {
            fs::write(directory.join(name), b"fixture")?;
        }
        fs::write(
            directory.join(RECEIPT_FILE),
            serde_json::to_vec(&receipt(&data, &data.platforms[0], false))?,
        )?;
        let descriptor = root.join("godot_cef.gdextension");
        let extension_list = root.join("extension_list.cfg");
        let resource = "res://addons/godot_cef/godot_cef.gdextension";
        let others = b"res://addons/other/other.gdextension\n";
        fs::write(&extension_list, others)?;
        // Readable, but Windows refuses renaming or replacing this open list.
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&extension_list)?;
        fs::write(&descriptor, b"previous working runtime")?;
        assert!(finalize_extension_activation(&root, &data, &extension_list, resource).is_err());
        assert_eq!(fs::read(&descriptor)?, b"previous working runtime");
        assert_eq!(fs::read(&extension_list)?, others);
        fs::remove_file(&descriptor)?;
        assert!(finalize_extension_activation(&root, &data, &extension_list, resource).is_err());
        assert!(!descriptor.exists());
        assert_eq!(fs::read(&extension_list)?, others);
        drop(locked);
        assert!(fs::read_dir(&root)?.all(|entry| {
            entry.is_ok_and(|entry| !entry.file_name().to_string_lossy().starts_with(".gdcef-"))
        }));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn extension_registration_rejects_injected_paths_and_preserves_existing_lines() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-registration-input-test");
        fs::create_dir_all(&root)?;
        let list = root.join("extension_list.cfg");
        let resource = "res://addons/godot_cef/godot_cef.gdextension";
        let original = format!("res://other.gdextension\r\n  {resource}  \r\n");
        fs::write(&list, &original)?;
        assert_eq!(
            extension_registration(&list, resource)?,
            original.as_bytes()
        );
        assert!(
            extension_registration(&list, "res://addons/evil\nres://godot_cef.gdextension")
                .is_err()
        );
        assert!(extension_registration(&list, "res://../godot_cef.gdextension").is_err());
        assert_eq!(fs::read(&list)?, original.as_bytes());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn foreign_only_install_preserves_previous_active_descriptor() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-foreign-manifest-test");
        fs::create_dir_all(&root)?;
        let mut data = manifest();
        if host_target() == "x86_64-pc-windows-msvc" {
            data.platforms[0].target = "x86_64-unknown-linux-gnu".into();
        }
        let directory = platform_directory(&root, &data.platforms[0])?;
        fs::create_dir_all(directory.join("locales"))?;
        for name in ["gdcef.dll", "libcef.dll", "gdcef_helper.exe"] {
            fs::write(directory.join(name), b"fixture")?;
        }
        fs::write(
            directory.join(RECEIPT_FILE),
            serde_json::to_vec(&receipt(&data, &data.platforms[0], false))?,
        )?;
        fs::write(root.join("godot_cef.gdextension"), b"old working runtime")?;
        assert!(!defer_extension_activation(&root, &data)?);
        activate_extension(&root, &data)?;
        assert_eq!(
            fs::read(root.join("godot_cef.gdextension"))?,
            b"old working runtime"
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn descriptor_publication_preserves_conflicting_directories() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-descriptor-conflict-test");
        let descriptor = root.join("godot_cef.gdextension");
        fs::create_dir_all(&descriptor)?;
        fs::write(descriptor.join("keep"), b"unrelated data")?;
        assert!(replace_file(&descriptor, b"new descriptor").is_err());
        assert_eq!(fs::read(descriptor.join("keep"))?, b"unrelated data");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_publication_does_not_follow_links() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-descriptor-link-test");
        fs::create_dir_all(&root)?;
        let outside = root.join("outside");
        fs::write(&outside, b"keep")?;
        let descriptor = root.join("godot_cef.gdextension");
        std::os::unix::fs::symlink(&outside, &descriptor)?;
        assert!(replace_file(&descriptor, b"new descriptor").is_err());
        assert_eq!(fs::read(&outside)?, b"keep");
        assert!(fs::symlink_metadata(descriptor)?.file_type().is_symlink());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
