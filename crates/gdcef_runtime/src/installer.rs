use crate::{
    Manifest, Platform, Result, fail,
    helper::{reject_links, safe_relative, temporary_path},
    manifest::{RECEIPT_FILE, helper_payload, platform_directory, receipt, validate_files},
};
use download_cef::{CefFile, CefIndex};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default)]
pub enum InstallSource {
    #[default]
    Download,
    LocalDirectory(PathBuf),
    LocalArchive(PathBuf),
}

#[derive(Clone, Debug, Default)]
pub struct InstallOptions {
    pub source: InstallSource,
    pub cache_dir: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub target: String,
    pub message: String,
    pub downloaded: u64,
    pub total: Option<u64>,
}

struct WorkDirectory(PathBuf);
impl Drop for WorkDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(fail("Installation cancelled"))
    } else {
        Ok(())
    }
}

fn report(progress: &Sender<Progress>, target: &str, message: impl Into<String>) {
    let _ = progress.send(Progress {
        target: target.to_owned(),
        message: message.into(),
        downloaded: 0,
        total: None,
    });
}

fn cef_targets(target: &str) -> Vec<&str> {
    if target == "universal-apple-darwin" {
        vec!["aarch64-apple-darwin", "x86_64-apple-darwin"]
    } else {
        vec![target]
    }
}

fn archive_platform(target: &str) -> Result<&'static str> {
    match target {
        "x86_64-pc-windows-msvc" => Ok("windows64"),
        "aarch64-pc-windows-msvc" => Ok("windowsarm64"),
        "x86_64-unknown-linux-gnu" => Ok("linux64"),
        "aarch64-unknown-linux-gnu" => Ok("linuxarm64"),
        "x86_64-apple-darwin" => Ok("macosx64"),
        "aarch64-apple-darwin" => Ok("macosarm64"),
        _ => Err(fail(format!("Unsupported CEF target: {target}"))),
    }
}

fn validate_archive_name(name: &str, target: &str, version: &str) -> Result<()> {
    if !name.starts_with(&format!("cef_binary_{version}+"))
        || !name.ends_with(&format!("_{}_minimal.tar.bz2", archive_platform(target)?))
    {
        return Err(fail(format!(
            "CEF archive must match version {version} and target {target}: {name}"
        )));
    }
    safe_relative(name)?;
    if name.contains('/') {
        return Err(fail("CEF archive name must not contain directories"));
    }
    Ok(())
}

fn hash_file(path: &Path, cancel: &AtomicBool) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = sha1_smol::Sha1::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        check_cancel(cancel)?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash.digest().to_string())
}

fn network_agent(index: bool) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_resolve(Some(Duration::from_secs(10)))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_per_read(Some(Duration::from_secs(15)))
        .timeout_per_write(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(15)))
        .timeout_recv_body(Some(Duration::from_secs(if index { 45 } else { 60 * 60 })))
        .build()
        .into()
}

fn download_index(cancel: &AtomicBool) -> Result<CefIndex> {
    let agent = network_agent(true);
    let mut response = agent
        .get(&format!("{}/index.json", download_cef::DEFAULT_CDN_URL))
        .call()
        .map_err(|error| fail(error.to_string()))?;
    let mut input = response.body_mut().as_reader();
    let mut bytes = Vec::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        check_cancel(cancel)?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > 64 * 1024 * 1024 {
            return Err(fail("CEF index exceeds the size limit"));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    check_cancel(cancel)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn download_archive(
    target: &str,
    version: &str,
    cache: &Path,
    cancel: &AtomicBool,
    progress: &Sender<Progress>,
) -> Result<PathBuf> {
    report(progress, target, "Resolving the pinned CEF archive");
    check_cancel(cancel)?;
    let index = download_index(cancel)?;
    let metadata = index
        .platform(target)
        .and_then(|platform| platform.version(version))
        .and_then(|version| version.minimal())
        .map_err(|error| fail(error.to_string()))?;
    validate_archive_name(&metadata.name, target, version)?;
    fs::create_dir_all(cache)?;
    let destination = cache.join(&metadata.name);
    if destination.is_file() && hash_file(&destination, cancel)? == metadata.sha1 {
        report(progress, target, "Using verified cached CEF archive");
        return Ok(destination);
    }
    let temporary = temporary_path(cache, "cef-download");
    let result = (|| -> Result<()> {
        let agent = network_agent(false);
        let url = format!("{}/{}", download_cef::DEFAULT_CDN_URL, metadata.name);
        let mut response = agent
            .get(&url)
            .call()
            .map_err(|error| fail(error.to_string()))?;
        let total = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let mut input = response.body_mut().as_reader();
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        let mut hash = sha1_smol::Sha1::new();
        let mut buffer = vec![0_u8; 256 * 1024];
        let mut downloaded = 0_u64;
        let mut last_report = Instant::now();
        loop {
            check_cancel(cancel)?;
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
            hash.update(&buffer[..count]);
            downloaded += count as u64;
            if last_report.elapsed() >= Duration::from_millis(100) {
                let _ = progress.send(Progress {
                    target: target.to_owned(),
                    message: "Downloading CEF".to_owned(),
                    downloaded,
                    total,
                });
                last_report = Instant::now();
            }
        }
        check_cancel(cancel)?;
        if hash.digest().to_string() != metadata.sha1 {
            return Err(fail("Downloaded CEF archive checksum mismatch"));
        }
        output.sync_all()?;
        drop(output);
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(destination)
}

fn local_directory(root: &Path, target: &str, version: &str) -> Result<PathBuf> {
    let short = download_cef::OsAndArch::try_from(target)
        .map_err(|error| fail(error.to_string()))?
        .to_string();
    for path in [root.join(target), root.join(short), root.to_path_buf()] {
        let metadata = path.join("archive.json");
        if metadata.is_file() {
            let archive: CefFile = serde_json::from_slice(&fs::read(metadata)?)?;
            if validate_archive_name(&archive.name, target, version).is_ok() {
                return Ok(path);
            }
        }
    }
    Err(fail(format!(
        "No extracted CEF {version} for {target} with matching archive.json in {}",
        root.display()
    )))
}

fn local_archive(path: &Path, target: &str, version: &str) -> Result<PathBuf> {
    if let Some(name) = path.file_name().and_then(|value| value.to_str())
        && path.is_file()
        && validate_archive_name(name, target, version).is_ok()
    {
        return Ok(path.to_path_buf());
    }
    let parent = if path.is_dir() {
        path
    } else {
        path.parent().ok_or_else(|| fail("Archive has no parent"))?
    };
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && validate_archive_name(&entry.file_name().to_string_lossy(), target, version).is_ok()
        {
            return Ok(entry.path());
        }
    }
    Err(fail(format!(
        "No CEF {version} archive for {target} in {}",
        parent.display()
    )))
}

fn obtain_cef(
    options: &InstallOptions,
    target: &str,
    version: &str,
    workspace: &Path,
    cancel: &AtomicBool,
    progress: &Sender<Progress>,
) -> Result<PathBuf> {
    if let InstallSource::LocalDirectory(path) = &options.source
        && let Ok(directory) = local_directory(path, target, version)
    {
        return Ok(directory);
    }
    let archive = match &options.source {
        InstallSource::Download => {
            let cache = options.cache_dir.clone().unwrap_or_else(|| {
                std::env::temp_dir()
                    .join("godot-cef-downloads")
                    .join(version)
            });
            download_archive(target, version, &cache, cancel, progress)?
        }
        InstallSource::LocalArchive(path) | InstallSource::LocalDirectory(path) => {
            local_archive(path, target, version)?
        }
    };
    check_cancel(cancel)?;
    report(progress, target, "Extracting the CEF archive");
    let destination = workspace.join(target);
    fs::create_dir_all(&destination)?;
    let extracted = extract_cef_archive(&archive, &destination, cancel)?;
    check_cancel(cancel)?;
    Ok(extracted)
}

struct CancellableReader<'a, R> {
    reader: R,
    cancel: &'a AtomicBool,
}

impl<R: Read> Read for CancellableReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("Installation cancelled"));
        }
        self.reader.read(buffer)
    }
}

fn extract_cef_archive(archive: &Path, destination: &Path, cancel: &AtomicBool) -> Result<PathBuf> {
    let decoder = bzip2::read::BzDecoder::new(fs::File::open(archive)?);
    unpack_cef(
        CancellableReader {
            reader: decoder,
            cancel,
        },
        destination,
        cancel,
    )?;
    check_cancel(cancel)?;
    let name = archive
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(|value| value.strip_suffix(".tar.bz2"))
        .ok_or_else(|| fail("Invalid CEF archive filename"))?;
    let source = destination.join(safe_relative(name)?);
    let release = source.join("Release");
    if !release.is_dir()
        || !release
            .canonicalize()?
            .starts_with(destination.canonicalize()?)
    {
        return Err(fail(
            "CEF archive does not contain a valid Release directory",
        ));
    }
    let output = destination.join("runtime");
    fs::rename(&release, &output)?;
    let resources = source.join("Resources");
    if resources.is_dir() {
        for entry in fs::read_dir(resources)? {
            check_cancel(cancel)?;
            let entry = entry?;
            fs::rename(entry.path(), output.join(entry.file_name()))?;
        }
    }
    for name in ["LICENSE.txt", "LICENSE", "CREDITS.html"] {
        if source.join(name).is_file() {
            fs::rename(source.join(name), output.join(name))?;
        }
    }
    Ok(output)
}

struct ArchiveLink {
    path: PathBuf,
    target: PathBuf,
    original: PathBuf,
    hard: bool,
}

fn link_target(path: &Path, target: &Path, hard: bool) -> Result<PathBuf> {
    let text = target
        .to_str()
        .ok_or_else(|| fail("CEF link target is not UTF-8"))?;
    if text.contains(['\\', ':', '\0']) || target.is_absolute() {
        return Err(fail("CEF link leaves its archive"));
    }
    let mut resolved = if hard {
        PathBuf::new()
    } else {
        path.parent()
            .ok_or_else(|| fail("CEF link has no parent"))?
            .to_path_buf()
    };
    for component in target.components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !resolved.pop() {
                    return Err(fail("CEF link leaves its archive"));
                }
            }
            _ => return Err(fail("CEF link leaves its archive")),
        }
    }
    if resolved.as_os_str().is_empty() {
        return Err(fail("CEF link points at the extraction root"));
    }
    Ok(resolved)
}

fn unpack_cef<R: Read>(reader: R, destination: &Path, cancel: &AtomicBool) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    let mut links = Vec::new();
    let mut total = 0_u64;
    let mut count = 0_usize;
    for entry in archive.entries()? {
        check_cancel(cancel)?;
        let mut entry = entry?;
        count += 1;
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| fail("CEF archive is too large"))?;
        if count > 100_000 || total > 8 * 1024 * 1024 * 1024 {
            return Err(fail("CEF archive exceeds extraction limits"));
        }
        let name = entry
            .path()?
            .to_str()
            .ok_or_else(|| fail("CEF archive path is not UTF-8"))?
            .to_owned();
        let path = safe_relative(&name)?;
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            let original = entry
                .link_name()?
                .ok_or_else(|| fail("CEF archive link has no target"))?
                .into_owned();
            let target = link_target(&path, &original, kind.is_hard_link())?;
            links.push(ArchiveLink {
                path,
                target,
                original,
                hard: kind.is_hard_link(),
            });
        } else if kind.is_file() || kind.is_dir() {
            if !entry.unpack_in(destination)? {
                return Err(fail("CEF archive path leaves extraction directory"));
            }
        } else {
            return Err(fail("Unsupported CEF archive entry type"));
        }
    }
    // No archive file is written through a symlink: links are processed only
    // after all regular entries have been extracted and their targets validated.
    while !links.is_empty() {
        check_cancel(cancel)?;
        let mut remaining = Vec::new();
        let mut completed = 0;
        #[cfg(not(unix))]
        let pending_paths: Vec<_> = links.iter().map(|link| link.path.clone()).collect();
        for link in links {
            reject_links(destination, &link.path)?;
            // Keep the original relative path until filesystem resolution. A
            // lexical `a/..` cancellation is not equivalent when `a` is itself
            // a symlink to a different directory depth.
            let target = if link.hard {
                destination.join(&link.target)
            } else {
                destination
                    .join(
                        link.path
                            .parent()
                            .ok_or_else(|| fail("CEF link has no parent"))?,
                    )
                    .join(&link.original)
            };
            if !target.exists() {
                remaining.push(link);
                continue;
            }
            if !target
                .canonicalize()?
                .starts_with(destination.canonicalize()?)
            {
                return Err(fail("CEF link leaves extraction directory"));
            }
            #[cfg(not(unix))]
            if target.is_dir()
                && pending_paths
                    .iter()
                    .any(|path| path.starts_with(&link.target) && !destination.join(path).exists())
            {
                remaining.push(link);
                continue;
            }
            let path = destination.join(&link.path);
            if fs::symlink_metadata(&path).is_ok() {
                return Err(fail("CEF link conflicts with an existing archive entry"));
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            if link.hard {
                if !fs::symlink_metadata(&target)?.file_type().is_file() {
                    return Err(fail("CEF hard link must point to a regular file"));
                }
                fs::hard_link(target, path)?;
            } else {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&link.original, path)?;
                #[cfg(not(unix))]
                {
                    let _ = &link.original;
                    // Windows extraction must also work without symlink
                    // privileges. Materialize only safe, already complete
                    // targets; macOS signs this layout after installation.
                    if link.path.starts_with(&link.target) {
                        return Err(fail("Cannot materialize a recursive CEF directory link"));
                    }
                    copy_tree(&target, &path, destination, cancel)?;
                }
            }
            completed += 1;
        }
        if completed == 0 {
            return Err(fail("CEF archive contains dangling or cyclic links"));
        }
        links = remaining;
    }
    Ok(())
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    source_root: &Path,
    cancel: &AtomicBool,
) -> Result<()> {
    check_cancel(cancel)?;
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        let link = fs::read_link(source)?;
        let resolved = source.canonicalize()?;
        if link.is_absolute() || !resolved.starts_with(source_root.canonicalize()?) {
            return Err(fail(format!(
                "CEF symlink leaves the runtime: {}",
                source.display()
            )));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(link, destination)?;
        #[cfg(not(unix))]
        copy_tree(&resolved, destination, source_root, cancel)?;
    } else if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(
                &entry.path(),
                &destination.join(entry.file_name()),
                source_root,
                cancel,
            )?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
        fs::set_permissions(destination, metadata.permissions())?;
    } else {
        return Err(fail("Unsupported file type in CEF runtime"));
    }
    Ok(())
}

const WINDOWS_ASSETS: &[&str] = &[
    "libcef.dll",
    "chrome_elf.dll",
    "d3dcompiler_47.dll",
    "dxcompiler.dll",
    "vk_swiftshader.dll",
    "vk_swiftshader_icd.json",
    "vulkan-1.dll",
    "icudtl.dat",
    "resources.pak",
    "chrome_100_percent.pak",
    "chrome_200_percent.pak",
    "v8_context_snapshot.bin",
    "bootstrap.exe",
    "bootstrapc.exe",
    "locales",
];
const LINUX_ASSETS: &[&str] = &[
    "libcef.so",
    "libvk_swiftshader.so",
    "libvulkan.so.1",
    "vk_swiftshader_icd.json",
    "icudtl.dat",
    "resources.pak",
    "chrome_100_percent.pak",
    "chrome_200_percent.pak",
    "v8_context_snapshot.bin",
    "chrome-sandbox",
    "locales",
];

fn stage_cef(source: &Path, target: &str, stage: &Path, cancel: &AtomicBool) -> Result<()> {
    if target.ends_with("-apple-darwin") {
        let name = if target.starts_with("aarch64") {
            "ARM64"
        } else {
            "X86_64"
        };
        copy_tree(&source.join("Chromium Embedded Framework.framework"), &stage.join(format!("Godot CEF.framework/Helpers/Godot CEF.app/Contents/Frameworks/Chromium Embedded Framework ({name}).framework")), source, cancel)?;
    } else {
        let assets = if target.ends_with("-pc-windows-msvc") {
            WINDOWS_ASSETS
        } else {
            LINUX_ASSETS
        };
        for asset in assets {
            copy_tree(&source.join(asset), &stage.join(asset), source, cancel)?;
        }
        // Recent Windows CEF builds include dxil.dll alongside dxcompiler.dll.
        if source.join("dxil.dll").is_file() {
            copy_tree(
                &source.join("dxil.dll"),
                &stage.join("dxil.dll"),
                source,
                cancel,
            )?;
        }
    }
    // Preserve upstream attribution without shipping the build-time SDK.
    for name in ["LICENSE.txt", "LICENSE", "CREDITS.html"] {
        if source.join(name).is_file() {
            let destination = stage.join("cef-licenses").join(target).join(name);
            copy_tree(&source.join(name), &destination, source, cancel)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn codesign(path: &Path, verify: bool) -> Result<()> {
    let mut command = std::process::Command::new("/usr/bin/codesign");
    if verify {
        command.args(["--verify", "--deep", "--strict"]);
    } else {
        command.args(["--force", "--sign", "-"]);
    }
    let output = command.arg(path).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(fail(format!(
            "macOS code signing failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

#[cfg(target_os = "macos")]
fn sign_mac(stage: &Path) -> Result<()> {
    let framework = stage.join("Godot CEF.framework");
    let app = framework.join("Helpers/Godot CEF.app");
    let frameworks = app.join("Contents/Frameworks");
    for arch in ["ARM64", "X86_64"] {
        let cef = frameworks.join(format!("Chromium Embedded Framework ({arch}).framework"));
        for entry in fs::read_dir(cef.join("Libraries"))? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "dylib")
            {
                codesign(&path, false)?;
            }
        }
        codesign(&cef, false)?;
    }
    // Helper apps are already signed in CI. Re-signing them would alter the
    // embedded payload and is unnecessary; seal only the containing bundles.
    codesign(&app, false)?;
    codesign(&framework, false)?;
    codesign(&framework, true)
}

pub(crate) fn verify_mac(directory: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        codesign(&directory.join("Godot CEF.framework"), true)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = directory;
        Ok(())
    }
}

fn collect_files(directory: &Path, prefix: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory.join(prefix))? {
        let entry = entry?;
        let path = prefix.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            collect_files(directory, &path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

fn same_file_bytes(first: &Path, second: &Path, cancel: &AtomicBool) -> Result<bool> {
    if fs::metadata(first)?.len() != fs::metadata(second)?.len() {
        return Ok(false);
    }
    let mut first = fs::File::open(first)?;
    let mut second = fs::File::open(second)?;
    let mut left = vec![0_u8; 256 * 1024];
    let mut right = vec![0_u8; 256 * 1024];
    loop {
        check_cancel(cancel)?;
        let count = first.read(&mut left)?;
        if count == 0 {
            return Ok(true);
        }
        second.read_exact(&mut right[..count])?;
        if left[..count] != right[..count] {
            return Ok(false);
        }
    }
}

/// Commit a completely staged runtime. Existing files are moved into a rollback
/// directory, and the installation receipt is the last file published.
fn commit_stage(
    stage: &Path,
    destination: &Path,
    platform: &Platform,
    cancel: &AtomicBool,
) -> Result<()> {
    check_cancel(cancel)?;
    let backup = temporary_path(destination, "runtime-backup");
    fs::create_dir_all(&backup)?;
    let backup = WorkDirectory(backup);
    let mut paths = Vec::new();
    if platform.target == "universal-apple-darwin" {
        // Publish a signed framework as one unit, preserving its symlinks and
        // resource seal. The independently loaded bootstrap is outside it.
        for entry in fs::read_dir(stage)? {
            paths.push(PathBuf::from(entry?.file_name()));
        }
    } else {
        collect_files(stage, Path::new(""), &mut paths)?;
    }
    paths.sort_by_key(|path| {
        (
            path.file_name().is_some_and(|name| name == RECEIPT_FILE),
            path.clone(),
        )
    });
    let library_relative = Path::new(&platform.library)
        .strip_prefix(&platform.directory)
        .map_err(|_| fail("Library outside platform directory"))?;
    let mut committed = Vec::new();
    let result = (|| -> Result<()> {
        for relative in paths {
            // Windows/Linux copies only provide a complete staging tree for
            // validation. Never rewrite the native library loaded by Godot.
            if platform.target != "universal-apple-darwin" && relative == library_relative {
                continue;
            }
            check_cancel(cancel)?;
            reject_links(destination, &relative)?;
            let source = stage.join(&relative);
            let output = destination.join(&relative);
            if output.is_file()
                && fs::symlink_metadata(&source)?.is_file()
                && same_file_bytes(&source, &output, cancel)?
            {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    crate::helper::set_mode(
                        &output,
                        fs::metadata(&source)?.permissions().mode() & 0o777,
                    )?;
                }
                continue;
            }
            if output.is_dir() && !source.is_dir() {
                return Err(fail(format!(
                    "A runtime file is occupied by a directory: {}",
                    output.display()
                )));
            }
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            let previous = backup.0.join(&relative);
            if output.exists() {
                if let Some(parent) = previous.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::rename(&output, &previous)?;
            }
            if let Err(error) = fs::rename(&source, &output) {
                if previous.exists()
                    && let Err(restore_error) = fs::rename(&previous, &output)
                {
                    // Include this entry in rollback so its backup is never
                    // discarded if restoring it also encounters an error.
                    committed.push(relative);
                    return Err(fail(format!(
                        "Cannot install runtime: {error}; cannot restore prior file: {restore_error}"
                    )));
                }
                return Err(error.into());
            }
            committed.push(relative);
        }
        Ok(())
    })();
    if result.is_err() {
        let mut rollback_error = None;
        for relative in committed.iter().rev() {
            let path = destination.join(relative);
            if path.exists() {
                let removal = if path.is_dir() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                if let Err(error) = removal {
                    rollback_error = Some(error);
                    continue;
                }
            }
            let previous = backup.0.join(relative);
            if previous.exists()
                && let Err(error) = fs::rename(previous, path)
            {
                rollback_error = Some(error);
            }
        }
        if let Some(error) = rollback_error {
            let retained = backup.0.clone();
            std::mem::forget(backup);
            return Err(fail(format!(
                "Installation rollback failed: {error}. Previous runtime files are retained in {}",
                retained.display()
            )));
        }
    }
    result
}

fn install_platform(
    addon: &Path,
    manifest: &Manifest,
    platform: &Platform,
    options: &InstallOptions,
    cancel: &AtomicBool,
    progress: &Sender<Progress>,
) -> Result<()> {
    let destination = platform_directory(addon, platform)?;
    if !destination.is_dir() {
        return Err(fail(
            "The platform native libraries are missing from this plugin package",
        ));
    }
    let current = crate::statuses(addon, manifest)
        .into_iter()
        .find(|state| state.target == platform.target);
    if current.as_ref().is_some_and(|state| state.installed) {
        if crate::manifest::verify_runtime(addon, manifest, platform).is_ok() {
            report(progress, &platform.target, "Runtime is already installed");
            return Ok(());
        }
        report(progress, &platform.target, "Repairing runtime helper files");
    }
    let workspace = temporary_path(&destination, "runtime-stage");
    fs::create_dir_all(&workspace)?;
    let workspace = WorkDirectory(workspace);
    let stage = workspace.0.join("install");
    fs::create_dir_all(&stage)?;
    let payload = helper_payload(addon, platform)?;
    report(progress, &platform.target, "Restoring the embedded helper");
    crate::ensure_helper_payload(&payload, &platform.helper.sha256, &stage)?;
    // Stage the core library as well: macOS signs the containing framework only
    // after helper and CEF contents are complete. The bootstrap stays untouched.
    let library = Path::new(&platform.library)
        .strip_prefix(&platform.directory)
        .map_err(|_| fail("Library outside platform directory"))?;
    copy_tree(
        &addon.join(&platform.library),
        &stage.join(library),
        &destination,
        cancel,
    )?;
    if platform.target == "universal-apple-darwin"
        && current
            .as_ref()
            .is_some_and(|state| state.needs_signing || state.installed)
    {
        report(
            progress,
            &platform.target,
            "Preparing the downloaded macOS runtime for signing",
        );
        for arch in ["ARM64", "X86_64"] {
            let relative = format!(
                "Godot CEF.framework/Helpers/Godot CEF.app/Contents/Frameworks/Chromium Embedded Framework ({arch}).framework"
            );
            copy_tree(
                &destination.join(&relative),
                &stage.join(&relative),
                &destination,
                cancel,
            )?;
        }
        if destination.join("cef-licenses").is_dir() {
            copy_tree(
                &destination.join("cef-licenses"),
                &stage.join("cef-licenses"),
                &destination,
                cancel,
            )?;
        }
    } else if current.as_ref().is_some_and(|state| state.installed) {
        stage_cef(&destination, &platform.target, &stage, cancel)?;
        if destination.join("cef-licenses").is_dir() {
            copy_tree(
                &destination.join("cef-licenses"),
                &stage.join("cef-licenses"),
                &destination,
                cancel,
            )?;
        }
    } else {
        for target in cef_targets(&platform.target) {
            let cef = obtain_cef(
                options,
                target,
                &manifest.cef_version,
                &workspace.0,
                cancel,
                progress,
            )?;
            report(progress, target, "Preparing runtime files");
            stage_cef(&cef, target, &stage, cancel)?;
        }
    }
    validate_files(&stage, platform)?;
    let signing_pending = platform.target == "universal-apple-darwin" && !cfg!(target_os = "macos");
    #[cfg(target_os = "macos")]
    if platform.target == "universal-apple-darwin" {
        report(progress, &platform.target, "Signing the macOS runtime");
        sign_mac(&stage)?;
    }
    let metadata = serde_json::to_vec_pretty(&receipt(manifest, platform, signing_pending))?;
    fs::write(stage.join(RECEIPT_FILE), metadata)?;
    check_cancel(cancel)?;
    report(
        progress,
        &platform.target,
        "Installing the verified runtime",
    );
    commit_stage(&stage, &destination, platform, cancel)?;
    report(
        progress,
        &platform.target,
        if signing_pending {
            "Downloaded; finish signing on macOS"
        } else {
            "Installed"
        },
    );
    Ok(())
}

/// Install selected platform runtimes on a worker thread. A cancelled or failed
/// stage leaves the previously installed runtime and extension descriptor intact.
pub fn install(
    addon: &Path,
    manifest: &Manifest,
    targets: &[String],
    options: &InstallOptions,
    cancel: Arc<AtomicBool>,
    progress: Sender<Progress>,
) -> Result<()> {
    manifest.validate()?;
    if targets.is_empty() {
        return Err(fail("Select at least one runtime platform"));
    }
    let addon = addon.canonicalize()?;
    let lock_path = addon.join(".runtime-install.lock");
    reject_links(&addon, Path::new(".runtime-install.lock"))?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.try_lock()
        .map_err(|_| fail("Another Godot CEF installation is in progress"))?;
    for target in targets {
        check_cancel(&cancel)?;
        let platform = manifest.platform(target)?;
        install_platform(&addon, manifest, platform, options, &cancel, &progress)?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::panic_in_result_fn)] // Test assertions deliberately fail the test harness.
mod tests {
    use super::*;

    #[test]
    fn archive_selection_requires_exact_cef_version_and_architecture() {
        assert!(
            validate_archive_name(
                "cef_binary_156.0.26+g123+chromium-156_windows64_minimal.tar.bz2",
                "x86_64-pc-windows-msvc",
                "156.0.26"
            )
            .is_ok()
        );
        assert!(
            validate_archive_name(
                "cef_binary_156.0.25+g123_windows64_minimal.tar.bz2",
                "x86_64-pc-windows-msvc",
                "156.0.26"
            )
            .is_err()
        );
        assert!(
            validate_archive_name(
                "cef_binary_156.0.26+g123_windowsarm64_minimal.tar.bz2",
                "x86_64-pc-windows-msvc",
                "156.0.26"
            )
            .is_err()
        );
        assert!(
            validate_archive_name(
                "../cef_binary_156.0.26+g123_windows64_minimal.tar.bz2",
                "x86_64-pc-windows-msvc",
                "156.0.26"
            )
            .is_err()
        );
    }

    #[test]
    fn cancellation_prevents_staged_files_from_becoming_visible() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-cancel-test");
        fs::create_dir_all(root.join("stage"))?;
        fs::create_dir_all(root.join("destination"))?;
        fs::write(root.join("stage/libcef.dll"), b"new")?;
        fs::write(root.join("destination/libcef.dll"), b"old")?;
        let platform = Platform {
            target: "x86_64-pc-windows-msvc".into(),
            directory: "bin/windows".into(),
            library: "bin/windows/gdcef.dll".into(),
            bootstrap: "bin/windows/bootstrap.dll".into(),
            helper: crate::EmbeddedArchive {
                offset: 0,
                length: 1,
                sha256: "0".repeat(64),
            },
            files: vec![],
            dirs: vec![],
        };
        let result = commit_stage(
            &root.join("stage"),
            &root.join("destination"),
            &platform,
            &AtomicBool::new(true),
        );
        assert!(result.is_err());
        assert_eq!(fs::read(root.join("destination/libcef.dll"))?, b"old");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn failed_commit_restores_previously_installed_files() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-rollback-test");
        fs::create_dir_all(root.join("stage/z"))?;
        fs::create_dir_all(root.join("destination"))?;
        fs::write(root.join("stage/a"), b"new")?;
        fs::write(root.join("stage/z/b"), b"new")?;
        fs::write(root.join("destination/a"), b"old")?;
        fs::write(root.join("destination/z"), b"keep")?;
        let platform = Platform {
            target: "x86_64-pc-windows-msvc".into(),
            directory: "bin/windows".into(),
            library: "bin/windows/gdcef.dll".into(),
            bootstrap: "bin/windows/bootstrap.dll".into(),
            helper: crate::EmbeddedArchive {
                offset: 0,
                length: 1,
                sha256: "0".repeat(64),
            },
            files: vec![],
            dirs: vec![],
        };
        assert!(
            commit_stage(
                &root.join("stage"),
                &root.join("destination"),
                &platform,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert_eq!(fs::read(root.join("destination/a"))?, b"old");
        assert_eq!(fs::read(root.join("destination/z"))?, b"keep");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn repair_preserves_identical_dll_while_windows_prevents_its_replacement() -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        let root = temporary_path(&std::env::temp_dir(), "gdcef-locked-repair-test");
        fs::create_dir_all(root.join("stage"))?;
        fs::create_dir_all(root.join("destination"))?;
        fs::write(root.join("stage/libcef.dll"), b"unchanged")?;
        fs::write(root.join("destination/libcef.dll"), b"unchanged")?;
        fs::write(root.join("stage/gdcef_helper.exe"), b"restored helper")?;
        let mapped = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(root.join("destination/libcef.dll"))?;
        let platform = Platform {
            target: "x86_64-pc-windows-msvc".into(),
            directory: "bin/windows".into(),
            library: "bin/windows/gdcef.dll".into(),
            bootstrap: "bin/windows/bootstrap.dll".into(),
            helper: crate::EmbeddedArchive {
                offset: 0,
                length: 1,
                sha256: "0".repeat(64),
            },
            files: vec![],
            dirs: vec![],
        };
        commit_stage(
            &root.join("stage"),
            &root.join("destination"),
            &platform,
            &AtomicBool::new(false),
        )?;
        assert_eq!(
            fs::read(root.join("destination/gdcef_helper.exe"))?,
            b"restored helper"
        );
        drop(mapped);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    fn fixture(root: &Path) -> Result<(Manifest, InstallOptions)> {
        let source = root.join("offline-cef");
        fs::create_dir_all(source.join("locales"))?;
        for file in WINDOWS_ASSETS.iter().filter(|name| **name != "locales") {
            fs::write(source.join(file), b"cef fixture")?;
        }
        fs::write(source.join("locales/en-US.pak"), b"locale")?;
        fs::write(source.join("archive.json"), br#"{"type":"minimal","name":"cef_binary_156.0.26+g123_windows64_minimal.tar.bz2","sha1":"unused-for-local-directory"}"#)?;
        let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut archive = tar::Builder::new(gzip);
        let mut header = tar::Header::new_gnu();
        header.set_size(6);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append_data(&mut header, "gdcef_helper.exe", &b"helper"[..])?;
        let payload = archive.into_inner()?.finish()?;
        let directory = "bin/x86_64-pc-windows-msvc/2.1.3-cef-156.0.26";
        fs::create_dir_all(root.join(directory))?;
        let library = format!("{directory}/gdcef.dll");
        let bootstrap = format!("{directory}/gdcef_bootstrap.dll");
        let mut native = b"FAKE-PE".to_vec();
        let offset = native.len() as u64;
        native.extend_from_slice(&payload);
        fs::write(root.join(&library), native)?;
        fs::write(root.join(&bootstrap), b"bootstrap")?;
        let platform = Platform {
            target: "x86_64-pc-windows-msvc".into(),
            directory: directory.into(),
            library,
            bootstrap,
            helper: crate::EmbeddedArchive {
                offset,
                length: payload.len() as u64,
                sha256: crate::sha256(&payload),
            },
            files: WINDOWS_ASSETS
                .iter()
                .filter(|name| **name != "locales")
                .map(|name| (*name).to_string())
                .chain(["gdcef.dll".to_owned(), "gdcef_helper.exe".to_owned()])
                .collect(),
            dirs: vec!["locales".into()],
        };
        Ok((
            Manifest {
                schema_version: 1,
                plugin_version: "2.1.3".into(),
                cef_version: "156.0.26".into(),
                platforms: vec![platform],
            },
            InstallOptions {
                source: InstallSource::LocalDirectory(source),
                cache_dir: None,
            },
        ))
    }

    #[test]
    fn offline_install_defers_activation_and_is_repeatable() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-offline-test");
        let (manifest, options) = fixture(&root)?;
        let target = manifest.platforms[0].target.clone();
        let (sender, _receiver) = std::sync::mpsc::channel();
        install(
            &root,
            &manifest,
            std::slice::from_ref(&target),
            &options,
            Arc::new(AtomicBool::new(false)),
            sender.clone(),
        )?;
        assert!(crate::statuses(&root, &manifest)[0].installed);
        assert!(!root.join("godot_cef.gdextension").exists());
        assert_eq!(
            fs::read(root.join(&manifest.platforms[0].bootstrap))?,
            b"bootstrap"
        );
        install(
            &root,
            &manifest,
            &[target],
            &options,
            Arc::new(AtomicBool::new(false)),
            sender,
        )?;
        let helper = root
            .join(&manifest.platforms[0].directory)
            .join("gdcef_helper.exe");
        fs::remove_file(&helper)?;
        crate::manifest::verify_runtime(&root, &manifest, &manifest.platforms[0])?;
        assert_eq!(fs::read(helper)?, b"helper");
        let result = crate::prepare_export(&root, &manifest, &manifest.platforms[0].target);
        if crate::host_target() == "x86_64-pc-windows-msvc" {
            assert!(result.is_err_and(|error| error.to_string().contains("Restart the editor")));
            assert!(!root.join("godot_cef.gdextension").exists());
            crate::activate_extension(&root, &manifest)?;
            crate::prepare_export(&root, &manifest, &manifest.platforms[0].target)?;
        } else {
            assert!(result.is_err());
        }
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn wrong_offline_cef_version_does_not_activate_or_publish_runtime() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-version-test");
        let (manifest, options) = fixture(&root)?;
        fs::write(root.join("offline-cef/archive.json"), br#"{"type":"minimal","name":"cef_binary_155.0.1+g123_windows64_minimal.tar.bz2","sha1":"unused"}"#)?;
        let (sender, _receiver) = std::sync::mpsc::channel();
        assert!(
            install(
                &root,
                &manifest,
                &[manifest.platforms[0].target.clone()],
                &options,
                Arc::new(AtomicBool::new(false)),
                sender
            )
            .is_err()
        );
        assert!(!root.join("godot_cef.gdextension").exists());
        assert!(
            !root
                .join(&manifest.platforms[0].directory)
                .join("libcef.dll")
                .exists()
        );
        assert!(!crate::statuses(&root, &manifest)[0].installed);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn foreign_runtime_alone_cannot_pass_export_preflight() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-foreign-export-test");
        let (mut manifest, options) = fixture(&root)?;
        let (sender, _receiver) = std::sync::mpsc::channel();
        install(
            &root,
            &manifest,
            &[manifest.platforms[0].target.clone()],
            &options,
            Arc::new(AtomicBool::new(false)),
            sender,
        )?;
        if manifest.platforms[0].target == crate::host_target() {
            manifest.platforms[0].target = "x86_64-unknown-linux-gnu".into();
        }
        let descriptor = root.join("godot_cef.gdextension");
        if descriptor.exists() {
            fs::remove_file(&descriptor)?;
        }
        assert!(crate::statuses(&root, &manifest)[0].installed);
        crate::manifest::verify_runtime(&root, &manifest, &manifest.platforms[0])?;
        let result = crate::prepare_export(&root, &manifest, &manifest.platforms[0].target);
        assert!(result.is_err_and(|error| error.to_string().contains("editor platform runtime")));
        assert!(!descriptor.exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    fn link_archive(target: &str) -> Result<Vec<u8>> {
        let mut archive = tar::Builder::new(Vec::new());
        let mut file = tar::Header::new_gnu();
        file.set_size(4);
        file.set_mode(0o644);
        file.set_cksum();
        archive.append_data(&mut file, "cef/Release/real/data", &b"data"[..])?;
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        link.set_mode(0o777);
        link.set_link_name(target)?;
        link.set_cksum();
        archive.append_data(&mut link, "cef/Release/alias", &b""[..])?;
        Ok(archive.into_inner()?)
    }

    #[test]
    fn cef_links_stay_inside_archive_and_work_without_windows_symlink_privileges() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-cef-links-test");
        fs::create_dir_all(&root)?;
        unpack_cef(
            std::io::Cursor::new(link_archive("real")?),
            &root,
            &AtomicBool::new(false),
        )?;
        assert_eq!(fs::read(root.join("cef/Release/alias/data"))?, b"data");
        #[cfg(unix)]
        assert!(
            fs::symlink_metadata(root.join("cef/Release/alias"))?
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(&root)?;
        fs::create_dir_all(&root)?;
        assert!(
            unpack_cef(
                std::io::Cursor::new(link_archive("../../../outside")?),
                &root,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn cancelled_extraction_stops_before_reading_archive() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-cef-cancel-test");
        fs::create_dir_all(&root)?;
        let cancel = AtomicBool::new(true);
        let reader = CancellableReader {
            reader: std::io::Cursor::new(link_archive("real")?),
            cancel: &cancel,
        };
        assert!(unpack_cef(reader, &root, &cancel).is_err());
        assert!(fs::read_dir(&root)?.next().is_none());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn chained_cef_links_cannot_escape_through_a_shallower_alias() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-chained-link-test");
        let destination = root.join("extract");
        fs::create_dir_all(&destination)?;
        fs::write(root.join("secret"), b"outside")?;
        let mut archive = tar::Builder::new(Vec::new());
        for name in ["cef/Release/deep/placeholder", "cef/Release/secret"] {
            let mut file = tar::Header::new_gnu();
            file.set_size(4);
            file.set_mode(0o644);
            file.set_cksum();
            archive.append_data(&mut file, name, &b"data"[..])?;
        }
        for (name, target) in [
            ("cef/Release/deep/shallow", "../.."),
            ("cef/Release/deep/escape", "shallow/../../secret"),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            header.set_link_name(target)?;
            header.set_cksum();
            archive.append_data(&mut header, name, &b""[..])?;
        }
        let result = unpack_cef(
            std::io::Cursor::new(archive.into_inner()?),
            &destination,
            &AtomicBool::new(false),
        );
        assert!(result.is_err());
        assert!(!destination.join("cef/Release/deep/escape").exists());
        assert_eq!(fs::read(root.join("secret"))?, b"outside");
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
