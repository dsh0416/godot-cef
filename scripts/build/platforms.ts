export type Variant = "full" | "store";
export interface Platform {
  target: string;
  feature: string;
  library: string;
  bootstrap: string;
  files: readonly string[];
  dirs: readonly string[];
}
export const macTarget = "universal-apple-darwin";
export const macTargets = ["aarch64-apple-darwin", "x86_64-apple-darwin"] as const;
export const windowsAssets = [
  "libcef.dll", "chrome_elf.dll", "d3dcompiler_47.dll", "dxcompiler.dll",
  "vk_swiftshader.dll", "vk_swiftshader_icd.json", "vulkan-1.dll", "icudtl.dat", "resources.pak",
  "chrome_100_percent.pak", "chrome_200_percent.pak", "v8_context_snapshot.bin", "bootstrap.exe", "bootstrapc.exe",
];
export const linuxAssets = [
  "libcef.so", "libvk_swiftshader.so", "libvulkan.so.1", "vk_swiftshader_icd.json",
  "icudtl.dat", "resources.pak", "chrome_100_percent.pak", "chrome_200_percent.pak",
  "v8_context_snapshot.bin", "chrome-sandbox",
];
export const macFrameworks = "Godot CEF.framework/Helpers/Godot CEF.app/Contents/Frameworks";
export const platforms: readonly Platform[] = [
  {
    target: macTarget, feature: "macos", library: "Godot CEF.framework/libgdcef.dylib", bootstrap: "libgdcef_bootstrap.dylib",
    files: ["Godot CEF.framework/Resources/Info.plist", ...["ARM64", "X86_64"].map(arch => `${macFrameworks}/Chromium Embedded Framework (${arch}).framework/Chromium Embedded Framework`)],
    dirs: ["ARM64", "X86_64"].flatMap(arch => ["Libraries", "Resources"].map(dir => `${macFrameworks}/Chromium Embedded Framework (${arch}).framework/${dir}`)),
  },
  ...["x86_64", "aarch64"].map(arch => ({
    target: `${arch}-pc-windows-msvc`, feature: `windows.${arch === "aarch64" ? "arm64" : arch}`,
    library: "gdcef.dll", bootstrap: "gdcef_bootstrap.dll", files: windowsAssets, dirs: ["locales"],
  })),
  ...["x86_64", "aarch64"].map(arch => ({
    target: `${arch}-unknown-linux-gnu`, feature: `linux.${arch === "aarch64" ? "arm64" : arch}`,
    library: "libgdcef.so", bootstrap: "libgdcef_bootstrap.so", files: linuxAssets, dirs: ["locales"],
  })),
];
// Both distributions support the same architectures. Store defers runtime downloads.
export function includes(_variant: Variant, _target: string): boolean { return true; }
export function variant(value = "full"): Variant {
  if (value !== "full" && value !== "store") throw new Error(`Unsupported variant: ${value}`);
  return value;
}
