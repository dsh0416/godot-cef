fn main() {
    // On Windows, we need to explicitly export NvOptimusEnablement and
    // AmdPowerXpressRequestHighPerformance so the NVIDIA/AMD drivers can find them
    // in the PE export table. Rust's #[no_mangle] + #[used] keeps the symbols in
    // the binary, but the MSVC linker does not add them to the export directory
    // for EXE targets by default (unlike __declspec(dllexport) in C/C++).
    //
    // Without these exports, the NVIDIA Optimus driver won't route gdcef_helper.exe
    // to the discrete GPU, causing cross-GPU shared texture handle failures on
    // laptops with hybrid graphics.
    // Build scripts are compiled for the host, so use Cargo's target cfg env var
    // instead of #[cfg(target_os = "...")] to detect the compilation target.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg=/EXPORT:NvOptimusEnablement,DATA");
        println!("cargo:rustc-link-arg=/EXPORT:AmdPowerXpressRequestHighPerformance,DATA");
    }
}
