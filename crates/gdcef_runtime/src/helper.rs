use crate::{Result, fail};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_HELPER_BYTES: u64 = 256 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn safe_relative(value: &str) -> Result<PathBuf> {
    if value.is_empty() || value.contains(['\\', ':', '\0']) {
        return Err(fail(format!("Invalid relative path: {value:?}")));
    }
    let path = Path::new(value);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(fail(format!(
            "Path leaves the runtime directory: {value:?}"
        )));
    }
    let clean: PathBuf = path
        .components()
        .filter(|part| matches!(part, Component::Normal(_)))
        .collect();
    if clean.as_os_str().is_empty() {
        return Err(fail("Empty runtime path"));
    }
    Ok(clean)
}

pub(crate) fn reject_links(root: &Path, relative: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            current.push(component.as_os_str());
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                let linked = metadata.file_type().is_symlink();
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    linked || metadata.file_attributes() & 0x400 != 0
                };
                if linked {
                    return Err(fail(format!(
                        "Refusing linked runtime path: {}",
                        current.display()
                    )));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) fn temporary_path(parent: &Path, prefix: &str) -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(".{prefix}-{}-{number}", std::process::id()))
}

struct HelperFile {
    path: PathBuf,
    bytes: Vec<u8>,
    mode: u32,
}

fn matches_file(path: &Path, bytes: &[u8]) -> Result<bool> {
    let metadata = fs::metadata(path)?;
    let executable = path.extension().is_some_and(|extension| extension == "exe");
    if !metadata.is_file()
        || (!executable && metadata.len() != bytes.len() as u64)
        || metadata.len() > bytes.len() as u64 + 16 * 1024 * 1024
    {
        return Ok(false);
    }
    let actual = fs::read(path)?;
    Ok(actual == bytes || (executable && same_signed_pe_image(bytes, &actual)))
}

struct PeImage {
    checksum: usize,
    certificate_directory: usize,
    image_end: usize,
    signed: bool,
}

impl PeImage {
    fn parse(bytes: &[u8]) -> Option<Self> {
        fn word(bytes: &[u8], offset: usize) -> Option<u16> {
            Some(u16::from_le_bytes(
                bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
            ))
        }
        fn dword(bytes: &[u8], offset: usize) -> Option<usize> {
            usize::try_from(u32::from_le_bytes(
                bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
            ))
            .ok()
        }

        if bytes.get(..2)? != b"MZ" {
            return None;
        }
        let pe = dword(bytes, 0x3c)?;
        if pe < 0x40 || bytes.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
            return None;
        }
        let optional = pe.checked_add(24)?;
        let sections = usize::from(word(bytes, pe.checked_add(6)?)?);
        let optional_end = optional.checked_add(usize::from(word(bytes, pe.checked_add(20)?)?))?;
        let checksum = optional.checked_add(64)?;
        let certificate_directory = optional.checked_add(144)?;
        // Distributed Windows helpers are x86_64/ARM64 PE32+ executables.
        if !matches!(word(bytes, pe.checked_add(4)?)?, 0x8664 | 0xaa64)
            || word(bytes, optional)? != 0x20b
            || sections == 0
            || optional_end > bytes.len()
            || certificate_directory.checked_add(8)? > optional_end
            || dword(bytes, optional.checked_add(108)?)? < 5
        {
            return None;
        }
        let certificate_start = dword(bytes, certificate_directory)?;
        let certificate_size = dword(bytes, certificate_directory.checked_add(4)?)?;
        let signed = certificate_start != 0 || certificate_size != 0;
        let image_end = if signed {
            if certificate_start % 8 != 0
                || certificate_size < 8
                || certificate_start.checked_add(certificate_size)? != bytes.len()
            {
                return None;
            }
            let mut position = certificate_start;
            while position < bytes.len() {
                let length = dword(bytes, position)?;
                if length < 8
                    || !matches!(word(bytes, position.checked_add(4)?)?, 0x100 | 0x200)
                    || word(bytes, position.checked_add(6)?)? != 2
                {
                    return None;
                }
                let end = position.checked_add(length)?;
                let padded_end = end.checked_add(7)? & !7;
                if !bytes.get(end..padded_end)?.iter().all(|byte| *byte == 0) {
                    return None;
                }
                position = padded_end;
            }
            if position != bytes.len() {
                return None;
            }
            certificate_start
        } else {
            bytes.len()
        };
        let section_table_end = optional_end.checked_add(sections.checked_mul(40)?)?;
        let headers_end = dword(bytes, optional.checked_add(60)?)?;
        if headers_end < section_table_end || headers_end > image_end {
            return None;
        }
        for index in 0..sections {
            let section = optional_end.checked_add(index.checked_mul(40)?)?;
            let size = dword(bytes, section.checked_add(16)?)?;
            let start = dword(bytes, section.checked_add(20)?)?;
            if size != 0 && (start < headers_end || start.checked_add(size)? > image_end) {
                return None;
            }
        }
        Some(Self {
            checksum,
            certificate_directory,
            image_end,
            signed,
        })
    }
}

/// Accept Authenticode signing without replacing the signed executable. This
/// checks unchanged image bytes, not signer trust. Only the PE checksum, security
/// directory and a bounded appended certificate table may differ. See Microsoft's
/// PE format, "Attribute Certificate Table" and "Certificate Data" sections:
/// https://learn.microsoft.com/en-us/windows/win32/debug/pe-format
fn same_signed_pe_image(expected: &[u8], actual: &[u8]) -> bool {
    let (Some(original), Some(signed)) = (PeImage::parse(expected), PeImage::parse(actual)) else {
        return false;
    };
    if !signed.signed
        || original.checksum != signed.checksum
        || original.certificate_directory != signed.certificate_directory
        || original.image_end > signed.image_end
    {
        return false;
    }
    if original.image_end != signed.image_end {
        // Signing an unsigned image may insert at most seven alignment zeros.
        let Some(aligned) = original.image_end.checked_add(7).map(|end| end & !7) else {
            return false;
        };
        if original.signed
            || aligned != signed.image_end
            || !actual[original.image_end..signed.image_end]
                .iter()
                .all(|byte| *byte == 0)
        {
            return false;
        }
    }
    let checksum = original.checksum;
    let directory = original.certificate_directory;
    expected[..checksum] == actual[..checksum]
        && expected[checksum + 4..directory] == actual[checksum + 4..directory]
        && expected[directory + 8..original.image_end] == actual[directory + 8..original.image_end]
}

fn unpack_payload(payload: &[u8], expected_sha256: &str) -> Result<Vec<HelperFile>> {
    if sha256(payload) != expected_sha256.to_ascii_lowercase() {
        return Err(fail("Embedded helper checksum mismatch"));
    }
    let mut archive = tar::Archive::new(GzDecoder::new(Cursor::new(payload)));
    let mut files = Vec::new();
    let mut total = 0_u64;
    let mut seen = std::collections::HashSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry
            .path()?
            .to_str()
            .ok_or_else(|| fail("Helper archive path is not UTF-8"))?
            .to_owned();
        let path = safe_relative(&name)?;
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            return Err(fail(format!("Unsupported helper archive entry: {name}")));
        }
        if !seen.insert(path.clone()) {
            return Err(fail(format!("Duplicate helper archive entry: {name}")));
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| fail("Helper archive is too large"))?;
        if total > MAX_HELPER_BYTES {
            return Err(fail("Helper archive is too large"));
        }
        let mode = entry.header().mode()? & 0o777;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        files.push(HelperFile { path, bytes, mode });
    }
    if files.is_empty() {
        return Err(fail("Embedded helper archive is empty"));
    }
    Ok(files)
}

/// Restore embedded helper files without replacing an existing different binary.
/// The complete archive is validated before any installation writes occur.
pub fn ensure_helper_payload(
    payload: &[u8],
    expected_sha256: &str,
    destination: &Path,
) -> Result<()> {
    let files = unpack_payload(payload, expected_sha256)?;
    for file in &files {
        reject_links(destination, &file.path)?;
        let path = destination.join(&file.path);
        if path.exists() && !matches_file(&path, &file.bytes)? {
            return Err(fail(format!(
                "Installed helper differs from this plugin version: {}",
                path.display()
            )));
        }
    }
    for file in files {
        let path = destination.join(&file.path);
        let parent = path
            .parent()
            .ok_or_else(|| fail("Helper path has no parent"))?;
        fs::create_dir_all(parent)?;
        if !path.exists() {
            let temp = temporary_path(parent, "gdcef-helper");
            let result = (|| -> Result<()> {
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)?;
                output.write_all(&file.bytes)?;
                output.sync_all()?;
                drop(output);
                set_mode(&temp, file.mode)?;
                // A hard link publishes the complete file without overwriting a
                // concurrently installed helper (including a running executable).
                match fs::hard_link(&temp, &path) {
                    Ok(()) => Ok(()),
                    Err(_) if path.is_file() && matches_file(&path, &file.bytes)? => Ok(()),
                    Err(error) => Err(error.into()),
                }
            })();
            let _ = fs::remove_file(&temp);
            result?;
        }
        ensure_executable_mode(&path, file.mode)?;
    }
    Ok(())
}

fn ensure_executable_mode(path: &Path, archive_mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let current = fs::metadata(path)?.permissions().mode();
        let required = current | (archive_mode & 0o111);
        // Existing verified helpers may be installed read-only (e.g. 0555).
        // Do not restore archive write bits or chmod an already runnable file.
        if current != required {
            fs::set_permissions(path, fs::Permissions::from_mode(required))?;
        }
    }
    #[cfg(not(unix))]
    let _ = (path, archive_mode);
    Ok(())
}

pub(crate) fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(path)?.permissions().mode() & 0o777 != mode & 0o777 {
            fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        }
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::panic_in_result_fn)] // Test assertions deliberately fail the test harness.
mod tests {
    use super::*;

    fn pe_image(machine: u16) -> Vec<u8> {
        let mut image = vec![0_u8; 0x401];
        image[..2].copy_from_slice(b"MZ");
        image[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        image[0x80..0x84].copy_from_slice(b"PE\0\0");
        image[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        image[0x86..0x88].copy_from_slice(&1_u16.to_le_bytes());
        image[0x94..0x96].copy_from_slice(&0xf0_u16.to_le_bytes());
        image[0x98..0x9a].copy_from_slice(&0x20b_u16.to_le_bytes());
        image[0xd4..0xd8].copy_from_slice(&0x200_u32.to_le_bytes());
        image[0x104..0x108].copy_from_slice(&16_u32.to_le_bytes());
        image[0x198..0x19c].copy_from_slice(&0x200_u32.to_le_bytes());
        image[0x19c..0x1a0].copy_from_slice(&0x200_u32.to_le_bytes());
        image[0x200..0x401].fill(0x5a);
        image
    }

    fn add_test_certificate(image: &[u8]) -> Result<Vec<u8>> {
        let mut signed = image.to_vec();
        signed.resize((signed.len() + 7) & !7, 0);
        let offset = u32::try_from(signed.len()).map_err(|_| fail("PE fixture is too large"))?;
        signed[0x128..0x12c].copy_from_slice(&offset.to_le_bytes());
        signed[0x12c..0x130].copy_from_slice(&24_u32.to_le_bytes());
        signed[0xd8..0xdc].copy_from_slice(&12345_u32.to_le_bytes());
        signed.extend_from_slice(&24_u32.to_le_bytes());
        signed.extend_from_slice(&0x200_u16.to_le_bytes());
        signed.extend_from_slice(&2_u16.to_le_bytes());
        signed.extend_from_slice(&[0x30; 16]);
        Ok(signed)
    }

    #[test]
    fn pe_signing_preserves_image_bytes_and_rejects_corrupt_certificate_layouts() -> Result<()> {
        for machine in [0x8664, 0xaa64] {
            let image = pe_image(machine);
            let signed = add_test_certificate(&image)?;
            assert!(same_signed_pe_image(&image, &signed));
            let mut resigned = signed.clone();
            resigned[0x41f] = 0x31;
            assert!(same_signed_pe_image(&signed, &resigned));
            for offset in [0x20, 0x100, 0x200, 0x400, 0x401] {
                let mut changed = signed.clone();
                changed[offset] ^= 1;
                assert!(
                    !same_signed_pe_image(&image, &changed),
                    "changed byte {offset:x}"
                );
            }
            for offset in [0x128, 0x12c, 0x408, 0x40c, 0x40e] {
                let mut changed = signed.clone();
                changed[offset..offset + 4].fill(0xff);
                assert!(
                    !same_signed_pe_image(&image, &changed),
                    "corrupt field {offset:x}"
                );
            }
            let mut overlay = signed.clone();
            overlay.extend_from_slice(b"unaccounted overlay");
            assert!(!same_signed_pe_image(&image, &overlay));
            let mut overlapping = signed;
            overlapping[0x128..0x12c].copy_from_slice(&0x200_u32.to_le_bytes());
            overlapping[0x12c..0x130].copy_from_slice(&0x220_u32.to_le_bytes());
            overlapping[0x200..0x204].copy_from_slice(&0x220_u32.to_le_bytes());
            overlapping[0x204..0x206].copy_from_slice(&0x200_u16.to_le_bytes());
            overlapping[0x206..0x208].copy_from_slice(&2_u16.to_le_bytes());
            assert!(PeImage::parse(&overlapping).is_none());
            assert!(!same_signed_pe_image(&image, &overlapping));
        }
        Ok(())
    }

    #[test]
    fn signed_windows_helper_is_preserved_without_accepting_changed_code() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-signed-helper-test");
        fs::create_dir_all(&root)?;
        let image = pe_image(0x8664);
        let signed = add_test_certificate(&image)?;
        let bytes = payload(&[("gdcef_helper.exe", &image)])?;
        let path = root.join("gdcef_helper.exe");
        fs::write(&path, &signed)?;
        ensure_helper_payload(&bytes, &sha256(&bytes), &root)?;
        assert_eq!(fs::read(&path)?, signed);
        let mut changed = signed;
        changed[0x200] ^= 1;
        fs::write(&path, changed)?;
        assert!(ensure_helper_payload(&bytes, &sha256(&bytes), &root).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn existing_helper_preserves_read_only_mode_and_only_adds_missing_execute_bits() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let root = temporary_path(&std::env::temp_dir(), "gdcef-readonly-helper-test");
        fs::create_dir_all(&root)?;
        let path = root.join("gdcef_helper");
        fs::write(&path, b"helper")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o555))?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o555))?;
        let archive = payload(&[("gdcef_helper", b"helper")])?;
        let result = ensure_helper_payload(&archive, &sha256(&archive), &root);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755))?;
        result?;
        let readonly_mode = fs::metadata(&path)?.permissions().mode() & 0o777;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;
        ensure_helper_payload(&archive, &sha256(&archive), &root)?;
        let repaired_mode = fs::metadata(&path)?.permissions().mode() & 0o777;
        fs::remove_dir_all(root)?;
        assert_eq!(readonly_mode, 0o555);
        assert_eq!(repaired_mode, 0o751);
        Ok(())
    }

    fn payload(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
        let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut archive = tar::Builder::new(gzip);
        for (path, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            archive.append_data(&mut header, path, *bytes)?;
        }
        Ok(archive.into_inner()?.finish()?)
    }

    #[test]
    fn helper_is_verified_before_writes_and_existing_files_are_preserved() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-helper-test");
        fs::create_dir_all(&root)?;
        let bytes = payload(&[("helper", b"first")])?;
        assert!(ensure_helper_payload(&bytes, "invalid", &root).is_err());
        assert!(!root.join("helper").exists());
        ensure_helper_payload(&bytes, &sha256(&bytes), &root)?;
        ensure_helper_payload(&bytes, &sha256(&bytes), &root)?;
        let changed = payload(&[("new", b"new"), ("helper", b"changed")])?;
        assert!(ensure_helper_payload(&changed, &sha256(&changed), &root).is_err());
        assert!(!root.join("new").exists());
        assert_eq!(fs::read(root.join("helper"))?, b"first");
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn paths_cannot_escape_or_use_platform_specific_prefixes() {
        for path in [
            "../helper",
            "/helper",
            "C:/helper",
            "a/../../helper",
            "a\\helper",
            ".",
            "",
        ] {
            assert!(safe_relative(path).is_err(), "{path}");
        }
        assert!(
            safe_relative("Godot CEF.framework/Helpers/test.app/Contents/MacOS/helper").is_ok()
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_destination_is_rejected() -> Result<()> {
        let root = temporary_path(&std::env::temp_dir(), "gdcef-symlink-test");
        fs::create_dir_all(root.join("outside"))?;
        std::os::unix::fs::symlink(root.join("outside"), root.join("linked"))?;
        let bytes = payload(&[("linked/helper", b"data")])?;
        assert!(ensure_helper_payload(&bytes, &sha256(&bytes), &root).is_err());
        assert!(!root.join("outside/helper").exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
