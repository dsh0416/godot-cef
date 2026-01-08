#[cfg(target_os = "macos")]
use std::{io::Error, path::PathBuf};

#[cfg(target_os = "macos")]
pub fn get_framework_path() -> Result<PathBuf, Error> {
    use process_path::get_executable_path;

    let dylib_path = get_executable_path().unwrap();

    match dylib_path.ends_with("Godot CEF") {
        true => {
            // main app
            // from: Godot CEF.app/Contents/MacOS/Godot CEF
            // to:   Godot CEF.app/Contents/Frameworks/Chromium Embedded Framework.framework
            dylib_path
                .join("../../Frameworks")
                .join("Chromium Embedded Framework.framework")
                .canonicalize()
        }
        false => {
            // helper app
            // from: Godot CEF.app/Contents/Frameworks/Godot CEF Helper.app/Contents/MacOS/Godot CEF Helper
            // to:   Godot CEF.app/Contents/Frameworks/Chromium Embedded Framework.framework
            dylib_path
                .join("../../../..")
                .join("Chromium Embedded Framework.framework")
                .canonicalize()
        }
    }
}
