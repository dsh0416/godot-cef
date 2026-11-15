use sha2::{Digest, Sha256};
use std::{env, error::Error, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-env-changed=GDCEF_HELPER_ARCHIVE");
    let output = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is missing")?);
    let archive = match env::var_os("GDCEF_HELPER_ARCHIVE") {
        Some(path) => {
            let path = PathBuf::from(path).canonicalize()?;
            println!("cargo:rerun-if-changed={}", path.display());
            let contents = fs::read(&path)?;
            if !contents.starts_with(&[0x1f, 0x8b]) {
                return Err(
                    "GDCEF_HELPER_ARCHIVE must point to a gzip-compressed tar archive".into(),
                );
            }
            contents
        }
        // Direct Cargo builds remain useful for contributors who already have a
        // development runtime. Distribution builds supply and verify this archive.
        None => Vec::new(),
    };
    let hash = Sha256::digest(&archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    fs::write(output.join("gdcef_helper.tar.gz"), &archive)?;
    fs::write(
        output.join("embedded_helper.rs"),
        format!(
            "#[used]\nstatic HELPER_ARCHIVE: [u8; {}] = *include_bytes!(concat!(env!(\"OUT_DIR\"), \"/gdcef_helper.tar.gz\"));\nconst HELPER_SHA256: &str = {hash:?};\n",
            archive.len()
        ),
    )?;
    Ok(())
}
