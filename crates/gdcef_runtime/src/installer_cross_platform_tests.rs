//! Exercise target selection through the real offline installer on every host.
#![allow(clippy::panic_in_result_fn)] // Assertions intentionally fail tests.

use super::*;

const CEF_VERSION: &str = "156.0.26";
const DESKTOP_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
];
const MAC_TARGET: &str = "universal-apple-darwin";
const MAC_HELPER: &str = "Godot CEF.framework/Helpers/Godot CEF.app/Contents/MacOS/Godot CEF";

struct Asset {
    path: String,
    bytes: Vec<u8>,
    mode: u32,
}

fn asset(target: &str, path: &str, mode: u32) -> Asset {
    Asset {
        path: path.to_owned(),
        bytes: format!("fixture for {target}: {path}").into_bytes(),
        mode,
    }
}

fn cef_assets(target: &str) -> Vec<Asset> {
    let mut assets = if target.ends_with("-apple-darwin") {
        [
            ("Chromium Embedded Framework", 0o755),
            ("Libraries/libEGL.dylib", 0o755),
            ("Resources/icudtl.dat", 0o644),
        ]
        .into_iter()
        .map(|(path, mode)| {
            asset(
                target,
                &format!("Chromium Embedded Framework.framework/{path}"),
                mode,
            )
        })
        .collect()
    } else {
        let names = if target.ends_with("-pc-windows-msvc") {
            WINDOWS_ASSETS
        } else {
            LINUX_ASSETS
        };
        let mut assets: Vec<_> = names
            .iter()
            .filter(|name| **name != "locales")
            .map(|name| {
                asset(
                    target,
                    name,
                    if *name == "chrome-sandbox" {
                        0o755
                    } else {
                        0o644
                    },
                )
            })
            .collect();
        assets.push(asset(target, "locales/en-US.pak", 0o644));
        if target.ends_with("-pc-windows-msvc") {
            assets.push(asset(target, "dxil.dll", 0o644));
        }
        assets
    };
    assets.push(asset(target, "LICENSE.txt", 0o644));
    assets
}

fn archive_name(target: &str) -> Result<String> {
    Ok(format!(
        "cef_binary_{CEF_VERSION}+gfixture_{}_minimal.tar.bz2",
        archive_platform(target)?
    ))
}

fn append_asset<W: Write>(archive: &mut tar::Builder<W>, path: &str, item: &Asset) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(item.bytes.len() as u64);
    header.set_mode(item.mode);
    header.set_cksum();
    archive.append_data(&mut header, path, item.bytes.as_slice())?;
    Ok(())
}

fn write_source(root: &Path, target: &str, compressed: bool) -> Result<PathBuf> {
    fs::create_dir_all(root)?;
    let name = archive_name(target)?;
    if compressed {
        let path = root.join(&name);
        let writer =
            bzip2::write::BzEncoder::new(fs::File::create(&path)?, bzip2::Compression::fast());
        let mut archive = tar::Builder::new(writer);
        let prefix = name.trim_end_matches(".tar.bz2");
        for item in cef_assets(target) {
            let path = if item.path == "LICENSE.txt" {
                format!("{prefix}/{}", item.path)
            } else if !target.ends_with("-apple-darwin")
                && (item.path.starts_with("locales/")
                    || item.path.ends_with(".pak")
                    || item.path.ends_with(".dat")
                    || item.path.ends_with(".bin"))
            {
                format!("{prefix}/Resources/{}", item.path)
            } else {
                format!("{prefix}/Release/{}", item.path)
            };
            append_asset(&mut archive, &path, &item)?;
        }
        archive.into_inner()?.finish()?;
        Ok(path)
    } else {
        let directory = root.join(target);
        for item in cef_assets(target) {
            let path = directory.join(item.path);
            fs::create_dir_all(
                path.parent()
                    .ok_or_else(|| fail("Fixture file has no parent"))?,
            )?;
            fs::write(&path, item.bytes)?;
            crate::helper::set_mode(&path, item.mode)?;
        }
        fs::write(
            directory.join("archive.json"),
            serde_json::to_vec(&serde_json::json!({
                "type": "minimal", "name": name, "sha1": "offline-fixture"
            }))?,
        )?;
        Ok(directory)
    }
}

fn helper_name(target: &str) -> &'static str {
    if target == MAC_TARGET {
        MAC_HELPER
    } else if target.ends_with("-pc-windows-msvc") {
        "gdcef_helper.exe"
    } else {
        "gdcef_helper"
    }
}

fn installed_asset_path(target: &str, path: &str) -> String {
    if path == "LICENSE.txt" {
        format!("cef-licenses/{target}/{path}")
    } else if target.ends_with("-apple-darwin") {
        let arch = if target.starts_with("aarch64") {
            "ARM64"
        } else {
            "X86_64"
        };
        path.replacen("Chromium Embedded Framework.framework", &format!("Godot CEF.framework/Helpers/Godot CEF.app/Contents/Frameworks/Chromium Embedded Framework ({arch}).framework"), 1)
    } else {
        path.to_owned()
    }
}

fn platform_fixture(addon: &Path, target: &str) -> Result<Platform> {
    let directory = format!("bin/{target}/2.1.3-cef-{CEF_VERSION}");
    let native_name = if target == MAC_TARGET {
        "Godot CEF.framework/libgdcef.dylib"
    } else if target.ends_with("-pc-windows-msvc") {
        "gdcef.dll"
    } else {
        "libgdcef.so"
    };
    let helper = asset(target, helper_name(target), 0o755);
    let writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut archive = tar::Builder::new(writer);
    append_asset(&mut archive, &helper.path, &helper)?;
    let payload = archive.into_inner()?.finish()?;
    let library = format!("{directory}/{native_name}");
    let bootstrap = format!("{directory}/bootstrap-library");
    let native = addon.join(&library);
    fs::create_dir_all(
        native
            .parent()
            .ok_or_else(|| fail("Fixture library has no parent"))?,
    )?;
    let mut image = b"native fixture".to_vec();
    let offset = image.len() as u64;
    image.extend_from_slice(&payload);
    fs::write(native, image)?;
    fs::write(addon.join(&bootstrap), b"bootstrap")?;
    let mut files = vec![native_name.to_owned(), helper.path];
    for cef_target in cef_targets(target) {
        files.extend(
            cef_assets(cef_target)
                .iter()
                .map(|item| installed_asset_path(cef_target, &item.path)),
        );
    }
    Ok(Platform {
        target: target.to_owned(),
        directory,
        library,
        bootstrap,
        helper: crate::EmbeddedArchive {
            offset,
            length: payload.len() as u64,
            sha256: crate::sha256(&payload),
        },
        files,
        dirs: vec!["cef-licenses".to_owned()],
    })
}

fn manifest(platforms: Vec<Platform>) -> Manifest {
    Manifest {
        schema_version: 1,
        plugin_version: "2.1.3".to_owned(),
        cef_version: CEF_VERSION.to_owned(),
        platforms,
    }
}

fn verify_assets(directory: &Path, target: &str) -> Result<()> {
    for item in cef_assets(target) {
        let path = directory.join(installed_asset_path(target, &item.path));
        assert_eq!(
            fs::read(&path)?,
            item.bytes,
            "wrong target contents for {}",
            path.display()
        );
        #[cfg(unix)]
        if item.mode & 0o111 != 0 {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path)?.permissions().mode() & 0o111,
                item.mode & 0o111
            );
        }
    }
    Ok(())
}

#[test]
fn windows_and_linux_targets_install_from_archives_and_directories_on_any_host() -> Result<()> {
    for compressed in [false, true] {
        let root = WorkDirectory(temporary_path(
            &std::env::temp_dir(),
            "gdcef-cross-platform-test",
        ));
        let addon = root.0.join("addon");
        let sources = root.0.join("cef-sources");
        let mut platforms = Vec::new();
        for target in DESKTOP_TARGETS {
            write_source(&sources, target, compressed)?;
            platforms.push(platform_fixture(&addon, target)?);
        }
        let manifest = manifest(platforms);
        let targets: Vec<String> = DESKTOP_TARGETS
            .iter()
            .map(|target| (*target).to_owned())
            .collect();
        let mut options = InstallOptions {
            source: if compressed {
                InstallSource::LocalArchive(sources)
            } else {
                InstallSource::LocalDirectory(sources)
            },
            cache_dir: None,
        };
        let (sender, _) = std::sync::mpsc::channel();
        install(
            &addon,
            &manifest,
            &targets,
            &options,
            Arc::new(AtomicBool::new(false)),
            sender.clone(),
        )?;
        assert!(
            crate::statuses(&addon, &manifest)
                .iter()
                .all(|status| status.installed)
        );
        assert!(!addon.join("godot_cef.gdextension").exists());
        for platform in &manifest.platforms {
            let directory = addon.join(&platform.directory);
            verify_assets(&directory, &platform.target)?;
            let helper = directory.join(helper_name(&platform.target));
            assert_eq!(
                fs::read(&helper)?,
                asset(&platform.target, helper_name(&platform.target), 0o755).bytes
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(fs::metadata(&helper)?.permissions().mode() & 0o111, 0o111);
            }
            fs::remove_file(helper)?;
        }
        options.source = InstallSource::LocalDirectory(root.0.join("unavailable-source"));
        install(
            &addon,
            &manifest,
            &targets,
            &options,
            Arc::new(AtomicBool::new(false)),
            sender,
        )?;
        assert!(
            crate::statuses(&addon, &manifest)
                .iter()
                .all(|status| status.installed)
        );
    }
    Ok(())
}

#[test]
fn paired_macos_archives_extract_and_stage_on_any_host_without_signing() -> Result<()> {
    let root = WorkDirectory(temporary_path(
        &std::env::temp_dir(),
        "gdcef-macos-stage-test",
    ));
    let sources = root.0.join("sources");
    let mut selected = None;
    for target in cef_targets(MAC_TARGET) {
        selected = Some(write_source(&sources, target, true)?);
    }
    let options = InstallOptions {
        source: InstallSource::LocalArchive(selected.ok_or_else(|| fail("No macOS fixtures"))?),
        cache_dir: None,
    };
    let (sender, _) = std::sync::mpsc::channel();
    let stage = root.0.join("stage");
    let cancel = AtomicBool::new(false);
    for target in cef_targets(MAC_TARGET) {
        let source = obtain_cef(
            &options,
            target,
            CEF_VERSION,
            &root.0.join("extract"),
            &cancel,
            &sender,
        )?;
        stage_cef(&source, target, &stage, &cancel)?;
        verify_assets(&stage, target)?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[test]
fn macos_cross_host_install_remains_pending_and_repeats_without_source() -> Result<()> {
    let root = WorkDirectory(temporary_path(
        &std::env::temp_dir(),
        "gdcef-macos-pending-test",
    ));
    let sources = root.0.join("sources");
    for target in cef_targets(MAC_TARGET) {
        write_source(&sources, target, true)?;
    }
    let addon = root.0.join("addon");
    let manifest = manifest(vec![platform_fixture(&addon, MAC_TARGET)?]);
    let mut options = InstallOptions {
        source: InstallSource::LocalArchive(sources),
        cache_dir: None,
    };
    let (sender, _) = std::sync::mpsc::channel();
    let targets = [MAC_TARGET.to_owned()];
    install(
        &addon,
        &manifest,
        &targets,
        &options,
        Arc::new(AtomicBool::new(false)),
        sender.clone(),
    )?;
    let directory = addon.join(&manifest.platforms[0].directory);
    let original_receipt = fs::read(directory.join(RECEIPT_FILE))?;
    options.source = InstallSource::LocalArchive(root.0.join("unavailable-source.tar.bz2"));
    install(
        &addon,
        &manifest,
        &targets,
        &options,
        Arc::new(AtomicBool::new(false)),
        sender,
    )?;
    let status = &crate::statuses(&addon, &manifest)[0];
    assert!(!status.installed && status.needs_signing);
    assert_eq!(fs::read(directory.join(RECEIPT_FILE))?, original_receipt);
    assert!(!addon.join("godot_cef.gdextension").exists());
    for target in cef_targets(MAC_TARGET) {
        verify_assets(&directory, target)?;
    }
    Ok(())
}
