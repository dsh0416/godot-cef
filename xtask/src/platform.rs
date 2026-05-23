pub struct PlatformSpec {
    pub target: &'static str,
    pub artifact_name: &'static str,
    pub required_files: &'static [&'static str],
    pub required_dirs: &'static [&'static str],
}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
pub struct RuntimeAssetSpec {
    pub cef_files: &'static [&'static str],
    pub cef_dirs: &'static [&'static str],
    pub deploy_files: &'static [&'static str],
    pub deploy_dirs: &'static [&'static str],
}

pub const MACOS_UNIVERSAL_TARGET: &str = "universal-apple-darwin";
pub const WINDOWS_X64_TARGET: &str = "x86_64-pc-windows-msvc";
pub const WINDOWS_ARM64_TARGET: &str = "aarch64-pc-windows-msvc";
pub const LINUX_X64_TARGET: &str = "x86_64-unknown-linux-gnu";
pub const LINUX_ARM64_TARGET: &str = "aarch64-unknown-linux-gnu";

const MACOS_REQUIRED_FILES: &[&str] = &["Godot CEF.framework", "Godot CEF.app"];
const WINDOWS_REQUIRED_FILES: &[&str] = &["gdcef.dll", "gdcef_helper.exe", "libcef.dll"];
const LINUX_REQUIRED_FILES: &[&str] = &["libgdcef.so", "gdcef_helper", "libcef.so"];
const LOCALES_DIR: &[&str] = &["locales"];
const NO_REQUIRED_DIRS: &[&str] = &[];

pub const PLATFORM_SPECS: &[PlatformSpec] = &[
    PlatformSpec {
        target: MACOS_UNIVERSAL_TARGET,
        artifact_name: "gdcef-universal-apple-darwin",
        required_files: MACOS_REQUIRED_FILES,
        required_dirs: NO_REQUIRED_DIRS,
    },
    PlatformSpec {
        target: WINDOWS_X64_TARGET,
        artifact_name: "gdcef-x86_64-pc-windows-msvc",
        required_files: WINDOWS_REQUIRED_FILES,
        required_dirs: LOCALES_DIR,
    },
    PlatformSpec {
        target: WINDOWS_ARM64_TARGET,
        artifact_name: "gdcef-aarch64-pc-windows-msvc",
        required_files: WINDOWS_REQUIRED_FILES,
        required_dirs: LOCALES_DIR,
    },
    PlatformSpec {
        target: LINUX_X64_TARGET,
        artifact_name: "gdcef-x86_64-unknown-linux-gnu",
        required_files: LINUX_REQUIRED_FILES,
        required_dirs: LOCALES_DIR,
    },
    PlatformSpec {
        target: LINUX_ARM64_TARGET,
        artifact_name: "gdcef-aarch64-unknown-linux-gnu",
        required_files: LINUX_REQUIRED_FILES,
        required_dirs: LOCALES_DIR,
    },
];

#[cfg(any(target_os = "windows", test))]
pub const WINDOWS_RUNTIME_ASSETS: RuntimeAssetSpec = RuntimeAssetSpec {
    cef_files: &[
        "libcef.dll",
        "chrome_elf.dll",
        "libEGL.dll",
        "libGLESv2.dll",
        "d3dcompiler_47.dll",
        "dxcompiler.dll",
        "dxil.dll",
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
    ],
    cef_dirs: LOCALES_DIR,
    deploy_files: &[
        "gdcef.dll",
        "gdcef_helper.exe",
        "libcef.dll",
        "chrome_elf.dll",
        "libEGL.dll",
        "libGLESv2.dll",
        "d3dcompiler_47.dll",
        "dxcompiler.dll",
        "dxil.dll",
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
    ],
    deploy_dirs: LOCALES_DIR,
};

#[cfg(any(target_os = "linux", test))]
pub const LINUX_RUNTIME_ASSETS: RuntimeAssetSpec = RuntimeAssetSpec {
    cef_files: &[
        "libcef.so",
        "libEGL.so",
        "libGLESv2.so",
        "libvk_swiftshader.so",
        "libvulkan.so.1",
        "vk_swiftshader_icd.json",
        "icudtl.dat",
        "resources.pak",
        "chrome_100_percent.pak",
        "chrome_200_percent.pak",
        "v8_context_snapshot.bin",
        "chrome-sandbox",
    ],
    cef_dirs: LOCALES_DIR,
    deploy_files: &[
        "libgdcef.so",
        "gdcef_helper",
        "libcef.so",
        "libEGL.so",
        "libGLESv2.so",
        "libvk_swiftshader.so",
        "libvulkan.so.1",
        "vk_swiftshader_icd.json",
        "icudtl.dat",
        "resources.pak",
        "chrome_100_percent.pak",
        "chrome_200_percent.pak",
        "v8_context_snapshot.bin",
        "chrome-sandbox",
    ],
    deploy_dirs: LOCALES_DIR,
};

#[cfg(test)]
pub fn platform_spec(target: &str) -> Option<&'static PlatformSpec> {
    PLATFORM_SPECS.iter().find(|spec| spec.target == target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn platform_targets_are_unique() {
        let mut targets = HashSet::new();

        for spec in PLATFORM_SPECS {
            assert!(
                targets.insert(spec.target),
                "duplicate target {}",
                spec.target
            );
        }
    }

    #[test]
    fn every_platform_has_an_artifact_name_and_required_files() {
        for spec in PLATFORM_SPECS {
            assert!(!spec.artifact_name.is_empty());
            assert!(!spec.required_files.is_empty());
        }
    }

    #[test]
    fn runtime_assets_include_pack_validation_requirements() {
        for target in [WINDOWS_X64_TARGET, WINDOWS_ARM64_TARGET] {
            let Some(spec) = platform_spec(target) else {
                assert!(platform_spec(target).is_some(), "windows spec should exist");
                continue;
            };
            for file in spec.required_files {
                assert!(WINDOWS_RUNTIME_ASSETS.deploy_files.contains(file));
            }
            for dir in spec.required_dirs {
                assert!(WINDOWS_RUNTIME_ASSETS.deploy_dirs.contains(dir));
            }
        }

        for target in [LINUX_X64_TARGET, LINUX_ARM64_TARGET] {
            let Some(spec) = platform_spec(target) else {
                assert!(platform_spec(target).is_some(), "linux spec should exist");
                continue;
            };
            for file in spec.required_files {
                assert!(LINUX_RUNTIME_ASSETS.deploy_files.contains(file));
            }
            for dir in spec.required_dirs {
                assert!(LINUX_RUNTIME_ASSETS.deploy_dirs.contains(dir));
            }
        }
    }
}
