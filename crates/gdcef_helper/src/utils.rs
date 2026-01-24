#[cfg(target_os = "macos")]
use std::{io::Error, path::PathBuf};

#[cfg(target_os = "macos")]
fn get_framework_name() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "Chromium Embedded Framework (ARM64).framework",
        "x86_64" => "Chromium Embedded Framework (X86_64).framework",
        arch => panic!("Unsupported architecture: {}", arch),
    }
}

#[cfg(target_os = "macos")]
pub fn get_framework_path() -> Result<PathBuf, Error> {
    use process_path::get_executable_path;

    let dylib_path = get_executable_path().unwrap();
    let framework_name = get_framework_name();

    match dylib_path.ends_with("Godot CEF") {
        true => {
            // main app
            // from: Godot CEF.app/Contents/MacOS/Godot CEF
            // to:   Godot CEF.app/Contents/Frameworks/Chromium Embedded Framework (ARM64|X86_64).framework
            dylib_path
                .join("../../Frameworks")
                .join(framework_name)
                .canonicalize()
        }
        false => {
            // helper app
            // from: Godot CEF.app/Contents/Frameworks/Godot CEF Helper.app/Contents/MacOS/Godot CEF Helper
            // to:   Godot CEF.app/Contents/Frameworks/Chromium Embedded Framework (ARM64|X86_64).framework
            dylib_path
                .join("../../../..")
                .join(framework_name)
                .canonicalize()
        }
    }
}
